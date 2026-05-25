//! Configuration types for the dedicated SGX runtime.

use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use clap::{Args, ValueEnum};

use crate::bootstrap::load_registered_instance_ids;

/// Bootstrap metadata filename.
pub const BOOTSTRAP_INFO_FILENAME: &str = "bootstrap.json";
/// Registered instance ids filename.
pub const REGISTERED_INFO_FILENAME: &str = "registered.json";
/// SGX private key filename.
pub const PRIV_KEY_FILENAME: &str = "priv.key";

const DEFAULT_RAIKO2_SGX_CONFIG_SUBDIR: &str = ".config/raiko2/sgx/config";
const DEFAULT_RAIKO2_SGX_SECRET_SUBDIR: &str = ".config/raiko2/sgx/secrets";
const DEFAULT_FORK: &str = "shasta";
const DEFAULT_LISTEN_ADDR: &str = "0.0.0.0:8080";
/// Native-mode fallback instance id used when no explicit id is provided.
pub const DEFAULT_NATIVE_INSTANCE_ID: u32 = 0xDEAD_C0DE;

/// Runtime execution mode for the dedicated SGX prover.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum RuntimeMode {
    /// Run inside Gramine and load SGX quote material.
    #[default]
    Tee,
    /// Skip TEE attestation and use the fixed native signer identity.
    Native,
}

/// Global SGX runtime directories.
#[derive(Clone, Debug, Args)]
pub struct GlobalOpts {
    /// Runtime mode used by the dedicated SGX prover.
    #[arg(long, env = "RAIKO2_SGX_MODE", value_enum, default_value_t = RuntimeMode::Tee)]
    pub mode: RuntimeMode,
    /// Directory containing bootstrap metadata.
    #[arg(long, default_value_os_t = default_config_dir())]
    pub config_dir: PathBuf,
    /// Directory containing sealed private key material.
    #[arg(long, default_value_os_t = default_secret_dir())]
    pub secret_dir: PathBuf,
}

impl Default for GlobalOpts {
    fn default() -> Self {
        Self {
            mode: RuntimeMode::Tee,
            config_dir: default_config_dir(),
            secret_dir: default_secret_dir(),
        }
    }
}

/// Serve mode options.
#[derive(Clone, Debug, Args)]
pub struct ServeOpts {
    /// Socket address to bind the server to.
    #[arg(long, default_value = DEFAULT_LISTEN_ADDR)]
    pub listen_addr: String,
    /// Fork name used to resolve a registered instance id when one is not supplied directly.
    #[arg(long, default_value = DEFAULT_FORK)]
    pub fork: String,
    /// Optional SGX instance id override.
    #[arg(long)]
    pub instance_id: Option<u32>,
}

impl Default for ServeOpts {
    fn default() -> Self {
        Self {
            listen_addr: DEFAULT_LISTEN_ADDR.to_string(),
            fork: DEFAULT_FORK.to_string(),
            instance_id: None,
        }
    }
}

/// Resolved runtime configuration for serving.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServiceConfig {
    /// Socket address bound by the server.
    pub listen_addr: String,
    /// Named fork used to resolve the instance id.
    pub fork: String,
    /// Registered SGX instance id used in produced proofs.
    pub instance_id: u32,
}

/// Default config directory for SGX bootstrap artifacts.
#[must_use]
pub fn default_config_dir() -> PathBuf {
    default_dir(DEFAULT_RAIKO2_SGX_CONFIG_SUBDIR)
}

/// Default secrets directory for the SGX private key.
#[must_use]
pub fn default_secret_dir() -> PathBuf {
    default_dir(DEFAULT_RAIKO2_SGX_SECRET_SUBDIR)
}

/// Resolve the serving configuration, preferring an explicit instance id and
/// falling back to the registered fork mapping created after bootstrap.
///
/// # Errors
///
/// Returns an error when the registered mapping cannot be loaded, does not contain the
/// requested fork, or resolves to a value that does not fit in `u32`.
pub fn resolve_service_config(
    global_opts: &GlobalOpts,
    serve_opts: &ServeOpts,
) -> Result<ServiceConfig> {
    let instance_id = if let Some(instance_id) = serve_opts.instance_id {
        instance_id
    } else if global_opts.mode == RuntimeMode::Native {
        DEFAULT_NATIVE_INSTANCE_ID
    } else {
        let registered =
            load_registered_instance_ids(&global_opts.config_dir).with_context(|| {
                format!(
                    "load registered instance ids from {}",
                    global_opts.config_dir.display()
                )
            })?;
        let resolved = registered.get(&serve_opts.fork).copied().ok_or_else(|| {
            anyhow!(
                "registered instance id for fork {:?} not found",
                serve_opts.fork
            )
        })?;
        u32::try_from(resolved).map_err(|_| {
            anyhow!(
                "registered instance id for fork {:?} overflows u32",
                serve_opts.fork
            )
        })?
    };

    Ok(ServiceConfig {
        listen_addr: serve_opts.listen_addr.clone(),
        fork: serve_opts.fork.clone(),
        instance_id,
    })
}

fn default_dir(subdir: &str) -> PathBuf {
    std::env::var_os("HOME")
        .map_or_else(|| PathBuf::from("."), PathBuf::from)
        .join(subdir)
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_NATIVE_INSTANCE_ID, GlobalOpts, RuntimeMode, ServeOpts, default_config_dir,
        default_secret_dir, resolve_service_config,
    };
    use crate::bootstrap::save_registered_instance_ids;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "raiko2-sgx-runtime-config-{name}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    #[test]
    fn default_config_dir_uses_raiko2_sgx_namespace() {
        let dir = default_config_dir();
        let raw = dir.to_string_lossy();
        assert!(raw.contains(".config/raiko2/sgx/config"), "{raw}");
    }

    #[test]
    fn default_secret_dir_uses_raiko2_sgx_namespace() {
        let dir = default_secret_dir();
        let raw = dir.to_string_lossy();
        assert!(raw.contains(".config/raiko2/sgx/secrets"), "{raw}");
    }

    #[test]
    fn resolve_service_config_prefers_explicit_instance_id() {
        let global_opts = GlobalOpts::default();
        let serve_opts = ServeOpts {
            instance_id: Some(7),
            ..ServeOpts::default()
        };

        let config = resolve_service_config(&global_opts, &serve_opts).expect("service config");
        assert_eq!(config.instance_id, 7);
    }

    #[test]
    fn resolve_service_config_uses_registered_fork_mapping() {
        let config_dir = temp_dir("registered");
        save_registered_instance_ids(
            &config_dir,
            &std::collections::BTreeMap::from([("shasta".to_string(), 3131899904)]),
        )
        .expect("save mapping");
        let global_opts = GlobalOpts {
            mode: RuntimeMode::Tee,
            config_dir,
            secret_dir: default_secret_dir(),
        };
        let serve_opts = ServeOpts::default();

        let config = resolve_service_config(&global_opts, &serve_opts).expect("service config");
        assert_eq!(config.instance_id, 3131899904);
    }

    #[test]
    fn resolve_service_config_uses_native_default_instance_id_without_registration() {
        let global_opts = GlobalOpts {
            mode: RuntimeMode::Native,
            config_dir: temp_dir("native-default"),
            secret_dir: default_secret_dir(),
        };
        let serve_opts = ServeOpts::default();

        let config = resolve_service_config(&global_opts, &serve_opts).expect("service config");
        assert_eq!(config.instance_id, DEFAULT_NATIVE_INSTANCE_ID);
    }
}
