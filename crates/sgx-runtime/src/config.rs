//! Configuration types for the dedicated TEE runtime.

use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use clap::{Args, ValueEnum};
use raiko2_primitives::ProofType;

use crate::bootstrap::load_registered_instance_ids;

/// Bootstrap metadata filename.
pub const BOOTSTRAP_INFO_FILENAME: &str = "bootstrap.json";
/// Registered instance ids filename.
pub const REGISTERED_INFO_FILENAME: &str = "registered.json";
/// TEE private key filename.
pub const PRIV_KEY_FILENAME: &str = "priv.key";

const DEFAULT_RAIKO2_SGX_CONFIG_SUBDIR: &str = ".config/raiko2/sgx/config";
const DEFAULT_RAIKO2_SGX_SECRET_SUBDIR: &str = ".config/raiko2/sgx/secrets";
const DEFAULT_RAIKO2_TDX_CONFIG_SUBDIR: &str = ".config/raiko2/tdx/config";
const DEFAULT_RAIKO2_TDX_SECRET_SUBDIR: &str = ".config/raiko2/tdx/secrets";
const DEFAULT_FORK: &str = "shasta";
const DEFAULT_LISTEN_ADDR: &str = "0.0.0.0:8080";
/// Native-mode fallback instance id used when no explicit id is provided.
pub const DEFAULT_NATIVE_INSTANCE_ID: u32 = 0xDEAD_C0DE;
/// Native-mode fallback instance id used by the dedicated TDX prover.
pub const DEFAULT_NATIVE_TDX_INSTANCE_ID: u32 = 0x7D00_C0DE;

/// TEE runtime flavor using the shared GuestInput replay server.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum RuntimeFlavor {
    /// Dedicated SGX provider.
    #[default]
    Sgx,
    /// Dedicated TDX provider.
    Tdx,
}

impl RuntimeFlavor {
    #[must_use]
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Sgx => "sgx",
            Self::Tdx => "tdx",
        }
    }
}

impl ServiceConfig {
    #[must_use]
    pub(crate) const fn proof_type(&self) -> ProofType {
        match self.flavor {
            RuntimeFlavor::Sgx => ProofType::Sgx,
            RuntimeFlavor::Tdx => ProofType::Tdx,
        }
    }
}

/// Runtime execution mode for the dedicated TEE prover.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum RuntimeMode {
    /// Run inside Gramine and load SGX quote material.
    #[default]
    Tee,
    /// Skip TEE attestation and use the fixed native signer identity.
    Native,
}

/// Global TEE runtime directories.
#[derive(Clone, Debug, Args)]
pub struct GlobalOpts {
    /// Runtime flavor exposed by this prover binary.
    #[arg(skip = RuntimeFlavor::Sgx)]
    pub flavor: RuntimeFlavor,
    /// Runtime mode used by the dedicated TEE prover.
    #[arg(skip = RuntimeMode::Tee)]
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
            flavor: RuntimeFlavor::Sgx,
            mode: RuntimeMode::Tee,
            config_dir: default_config_dir(),
            secret_dir: default_secret_dir(),
        }
    }
}

impl GlobalOpts {
    /// Re-scope parsed global options to a concrete runtime flavor.
    #[must_use]
    pub fn for_flavor(mut self, flavor: RuntimeFlavor) -> Self {
        let using_default_config_dir = self.config_dir == default_config_dir_for(self.flavor);
        let using_default_secret_dir = self.secret_dir == default_secret_dir_for(self.flavor);
        self.flavor = flavor;
        if using_default_config_dir {
            self.config_dir = default_config_dir_for(flavor);
        }
        if using_default_secret_dir {
            self.secret_dir = default_secret_dir_for(flavor);
        }
        self
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
    /// Optional TEE instance id override.
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
    /// TEE runtime flavor exposed by this prover service.
    pub flavor: RuntimeFlavor,
    /// Socket address bound by the server.
    pub listen_addr: String,
    /// Named fork used to resolve the instance id.
    pub fork: String,
    /// Registered TEE instance id used in produced proofs.
    pub instance_id: u32,
}

/// Default config directory for SGX bootstrap artifacts.
#[must_use]
pub fn default_config_dir() -> PathBuf {
    default_config_dir_for(RuntimeFlavor::Sgx)
}

/// Default secrets directory for the TEE private key.
#[must_use]
pub fn default_secret_dir() -> PathBuf {
    default_secret_dir_for(RuntimeFlavor::Sgx)
}

/// Default config directory for a TEE runtime flavor.
#[must_use]
pub fn default_config_dir_for(flavor: RuntimeFlavor) -> PathBuf {
    default_dir(match flavor {
        RuntimeFlavor::Sgx => DEFAULT_RAIKO2_SGX_CONFIG_SUBDIR,
        RuntimeFlavor::Tdx => DEFAULT_RAIKO2_TDX_CONFIG_SUBDIR,
    })
}

/// Default secrets directory for a TEE runtime flavor.
#[must_use]
pub fn default_secret_dir_for(flavor: RuntimeFlavor) -> PathBuf {
    default_dir(match flavor {
        RuntimeFlavor::Sgx => DEFAULT_RAIKO2_SGX_SECRET_SUBDIR,
        RuntimeFlavor::Tdx => DEFAULT_RAIKO2_TDX_SECRET_SUBDIR,
    })
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
        native_instance_id(global_opts.flavor)
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
        flavor: global_opts.flavor,
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

const fn native_instance_id(flavor: RuntimeFlavor) -> u32 {
    match flavor {
        RuntimeFlavor::Sgx => DEFAULT_NATIVE_INSTANCE_ID,
        RuntimeFlavor::Tdx => DEFAULT_NATIVE_TDX_INSTANCE_ID,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_NATIVE_INSTANCE_ID, DEFAULT_NATIVE_TDX_INSTANCE_ID, GlobalOpts, RuntimeFlavor,
        RuntimeMode, ServeOpts, ServiceConfig, default_config_dir, default_config_dir_for,
        default_secret_dir, default_secret_dir_for, resolve_service_config,
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
    fn tdx_flavor_defaults_use_raiko2_tdx_namespace() {
        let config_dir = default_config_dir_for(RuntimeFlavor::Tdx);
        let secret_dir = default_secret_dir_for(RuntimeFlavor::Tdx);

        assert!(
            config_dir
                .to_string_lossy()
                .contains(".config/raiko2/tdx/config"),
            "{}",
            config_dir.display()
        );
        assert!(
            secret_dir
                .to_string_lossy()
                .contains(".config/raiko2/tdx/secrets"),
            "{}",
            secret_dir.display()
        );
    }

    #[test]
    fn service_config_maps_runtime_flavor_to_proof_type() {
        let sgx = ServiceConfig {
            flavor: RuntimeFlavor::Sgx,
            ..ServiceConfig::default()
        };
        let tdx = ServiceConfig {
            flavor: RuntimeFlavor::Tdx,
            ..ServiceConfig::default()
        };

        assert_eq!(sgx.proof_type(), raiko2_primitives::ProofType::Sgx);
        assert_eq!(tdx.proof_type(), raiko2_primitives::ProofType::Tdx);
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
            flavor: RuntimeFlavor::Sgx,
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
            flavor: RuntimeFlavor::Sgx,
            mode: RuntimeMode::Native,
            config_dir: temp_dir("native-default"),
            secret_dir: default_secret_dir(),
        };
        let serve_opts = ServeOpts::default();

        let config = resolve_service_config(&global_opts, &serve_opts).expect("service config");
        assert_eq!(config.instance_id, DEFAULT_NATIVE_INSTANCE_ID);
    }

    #[test]
    fn resolve_service_config_uses_tdx_native_default_instance_id() {
        let global_opts = GlobalOpts {
            flavor: RuntimeFlavor::Tdx,
            mode: RuntimeMode::Native,
            config_dir: temp_dir("tdx-native-default"),
            secret_dir: default_secret_dir_for(RuntimeFlavor::Tdx),
        };
        let serve_opts = ServeOpts::default();

        let config = resolve_service_config(&global_opts, &serve_opts).expect("service config");
        assert_eq!(config.instance_id, DEFAULT_NATIVE_TDX_INSTANCE_ID);
    }
}
