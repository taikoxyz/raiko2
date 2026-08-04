use anyhow::{Result, bail};
use raiko2_runtime::{StartupCleanupMask, validate_scope_component};
use serde::{Deserialize, Deserializer, Serialize, de::Error as _};
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeStoreBackend {
    #[default]
    Memory,
    Gcs,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum StartupCleanupScope {
    Proof,
    Preflight,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PreflightCacheMode {
    #[default]
    Shared,
    Off,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeStoreConfig {
    pub backend: RuntimeStoreBackend,
    pub bucket: Option<String>,
    #[serde(default = "default_prefix")]
    pub prefix: String,
    #[serde(default)]
    pub allow_ephemeral: bool,
}

impl Default for RuntimeStoreConfig {
    fn default() -> Self {
        Self {
            backend: RuntimeStoreBackend::Memory,
            bucket: None,
            prefix: default_prefix(),
            allow_ephemeral: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeConfig {
    pub environment: String,
    pub namespace: String,
    #[serde(default)]
    pub preflight_cache: PreflightCacheMode,
    #[serde(default, deserialize_with = "deserialize_startup_cleanup")]
    pub startup_cleanup: Vec<StartupCleanupScope>,
    pub store: RuntimeStoreConfig,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            environment: "development".to_string(),
            namespace: "raiko2-development".to_string(),
            preflight_cache: PreflightCacheMode::Shared,
            startup_cleanup: Vec::new(),
            store: RuntimeStoreConfig::default(),
        }
    }
}

impl RuntimeConfig {
    pub fn validate(&self) -> Result<()> {
        validate_scope_component("runtime.environment", &self.environment)?;
        validate_scope_component("runtime.namespace", &self.namespace)?;
        self.startup_cleanup_mask()?;
        match self.store.backend {
            RuntimeStoreBackend::Memory => {
                if self.store.bucket.is_some() {
                    bail!("runtime.store.bucket is only valid for backend=gcs");
                }
                if !self.store.allow_ephemeral
                    && !matches!(self.environment.as_str(), "development" | "local" | "test")
                {
                    bail!(
                        "runtime.store.backend=memory requires runtime.store.allow_ephemeral=true outside development, local, or test environments"
                    );
                }
            }
            RuntimeStoreBackend::Gcs => {
                if self.store.allow_ephemeral {
                    bail!("runtime.store.allow_ephemeral is only valid for backend=memory");
                }
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

    pub fn startup_cleanup_mask(&self) -> Result<StartupCleanupMask> {
        let mut seen = HashSet::new();
        let mut mask = StartupCleanupMask::NONE;
        for scope in &self.startup_cleanup {
            if !seen.insert(*scope) {
                bail!("runtime.startup_cleanup contains duplicate scope {scope:?}");
            }
            mask = mask
                | match scope {
                    StartupCleanupScope::Proof => StartupCleanupMask::PROOF,
                    StartupCleanupScope::Preflight => StartupCleanupMask::PREFLIGHT,
                };
        }
        Ok(mask)
    }
}

fn deserialize_startup_cleanup<'de, D>(
    deserializer: D,
) -> std::result::Result<Vec<StartupCleanupScope>, D::Error>
where
    D: Deserializer<'de>,
{
    let scopes = Vec::<StartupCleanupScope>::deserialize(deserializer)?;
    let mut seen = HashSet::new();
    for scope in &scopes {
        if !seen.insert(*scope) {
            return Err(D::Error::custom(format!(
                "runtime.startup_cleanup contains duplicate scope {scope:?}"
            )));
        }
    }
    Ok(scopes)
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
    fn gcs_rejects_ephemeral_opt_in() {
        let config = RuntimeConfig {
            store: RuntimeStoreConfig {
                backend: RuntimeStoreBackend::Gcs,
                bucket: Some("runtime-state".into()),
                allow_ephemeral: true,
                ..RuntimeStoreConfig::default()
            },
            ..RuntimeConfig::default()
        };

        assert!(
            config
                .validate()
                .unwrap_err()
                .to_string()
                .contains("only valid for backend=memory")
        );
    }

    #[test]
    fn environment_and_namespace_are_distinct_required_scopes() {
        let config = RuntimeConfig {
            environment: "devnet".into(),
            namespace: "raiko2-devnet-a".into(),
            preflight_cache: PreflightCacheMode::Shared,
            startup_cleanup: Vec::new(),
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

    #[test]
    fn memory_store_is_allowed_for_deployed_environments_with_explicit_opt_in() {
        let config = RuntimeConfig {
            environment: "hoodi".into(),
            namespace: "raiko2-hoodi-ephemeral".into(),
            preflight_cache: PreflightCacheMode::Shared,
            startup_cleanup: Vec::new(),
            store: RuntimeStoreConfig {
                allow_ephemeral: true,
                ..RuntimeStoreConfig::default()
            },
        };

        config
            .validate()
            .expect("explicit ephemeral storage opt-in");
    }

    #[test]
    fn startup_cleanup_defaults_to_empty() {
        let config: RuntimeConfig = toml::from_str(
            r#"
                environment = "development"
                namespace = "raiko2-development"

                [store]
                backend = "memory"
            "#,
        )
        .expect("runtime config without startup cleanup parses");

        assert!(config.startup_cleanup.is_empty());
        assert_eq!(
            config.startup_cleanup_mask().expect("cleanup mask"),
            StartupCleanupMask::NONE
        );
    }

    #[test]
    fn startup_cleanup_parses_exact_scopes_and_serializes_lowercase() {
        let config: RuntimeConfig = toml::from_str(
            r#"
                environment = "development"
                namespace = "raiko2-development"
                startup_cleanup = ["proof", "preflight"]

                [store]
                backend = "memory"
            "#,
        )
        .expect("startup cleanup scopes parse");

        assert_eq!(
            config.startup_cleanup_mask().expect("cleanup mask"),
            StartupCleanupMask::ALL
        );
        let encoded = toml::to_string(&config).expect("serialize runtime config");
        assert!(encoded.contains("startup_cleanup = [\"proof\", \"preflight\"]"));
    }

    #[test]
    fn preflight_cache_defaults_to_shared_and_accepts_off() {
        assert_eq!(
            RuntimeConfig::default().preflight_cache,
            PreflightCacheMode::Shared
        );

        let config: RuntimeConfig = toml::from_str(
            r#"
                environment = "development"
                namespace = "raiko2-development"
                preflight_cache = "off"

                [store]
                backend = "memory"
            "#,
        )
        .expect("off preflight cache mode parses");

        assert_eq!(config.preflight_cache, PreflightCacheMode::Off);
        let encoded = toml::to_string(&config).expect("serialize runtime config");
        assert!(encoded.contains("preflight_cache = \"off\""));
    }

    #[test]
    fn startup_cleanup_rejects_duplicate_unknown_and_removed_fields() {
        for input in [
            r#"
                environment = "development"
                namespace = "raiko2-development"
                startup_cleanup = ["proof", "proof"]
                [store]
                backend = "memory"
            "#,
            r#"
                environment = "development"
                namespace = "raiko2-development"
                startup_cleanup = ["input"]
                [store]
                backend = "memory"
            "#,
            r#"
                environment = "development"
                namespace = "raiko2-development"
                reset_namespace_on_start = true
                [store]
                backend = "memory"
            "#,
        ] {
            assert!(toml::from_str::<RuntimeConfig>(input).is_err());
        }
    }
}
