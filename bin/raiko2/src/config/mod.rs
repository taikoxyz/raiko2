//! Configuration management for Raiko V2.

use crate::cli::Cli;
use anyhow::{Context, Result};
use raiko2_primitives::ProofType;
use serde::{Deserialize, Serialize};
use std::path::Path;

mod preflight;
mod prover;
mod queue;
mod rpc;
mod runtime;
mod server;
mod validation;

pub use preflight::PreflightConfig;
pub use prover::{ProverConfig, ZkAnyConfig, ZkAnyTargetConfig};
pub use queue::QueueConfig;
pub use raiko2_pipeline::{GuestSystem, RunnerKind};
pub use rpc::{BoundlessPairConfig, NetworkPairConfig, ResolvedNetworkPair, RpcConfig};
pub use runtime::{RuntimeConfig, RuntimeStoreBackend};
#[cfg(test)]
pub use server::{ServerAclConfig, ServerAclKey};
pub use server::{ServerAclFeature, ServerConfig};

#[cfg(test)]
use raiko2_pipeline::PipelineRoute;
#[cfg(test)]
use raiko2_provider::L2ProviderKind;

/// Full application configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub server: ServerConfig,
    pub rpc: RpcConfig,
    pub prover: ProverConfig,
    pub runtime: RuntimeConfig,
    #[serde(default)]
    pub queue: QueueConfig,
    #[serde(default)]
    pub preflight: PreflightConfig,
}

impl Config {
    /// Load configuration from CLI arguments and optional config file.
    pub fn load(cli: &Cli) -> Result<Self> {
        if std::env::var_os("RAIKO2_PROVER").is_some() {
            anyhow::bail!(
                "RAIKO2_PROVER is no longer supported; use prover routes in the config file or RAIKO2_PROVER_ROUTES"
            );
        }
        let mut config = if let Some(config_path) = &cli.config {
            Self::from_file(config_path)?
        } else {
            Self::default()
        };

        // Override with CLI arguments
        if let Some(host) = &cli.host {
            config.server.host.clone_from(host);
        }
        if let Some(port) = cli.port {
            config.server.port = port;
        }

        if let Some(l1_rpc) = &cli.l1_rpc {
            override_single_rpc_pair(&mut config.rpc, |pair| pair.l1_rpc = Some(l1_rpc.clone()))?;
        }
        if let Some(l2_rpc) = &cli.l2_rpc {
            override_single_rpc_pair(&mut config.rpc, |pair| {
                pair.l2_rpc = Some(l2_rpc.clone());
            })?;
        }
        if let Some(timeout_ms) = cli.rpc_timeout_ms {
            config.rpc.client.timeout_ms = timeout_ms;
        }
        if let Some(concurrency_limit) = cli.rpc_concurrency_limit {
            config.rpc.client.concurrency_limit = concurrency_limit;
        }
        if let Some(max_attempts) = cli.rpc_retry_max_attempts {
            config.rpc.client.retry.max_attempts = max_attempts;
        }
        if let Some(initial_backoff_ms) = cli.rpc_retry_initial_backoff_ms {
            config.rpc.client.retry.initial_backoff_ms = initial_backoff_ms;
        }
        if let Some(cu_per_second) = cli.rpc_retry_cu_per_second {
            config.rpc.client.retry.compute_units_per_second = cu_per_second;
        }

        if let Some(routes) = &cli.prover_routes {
            let routes = routes.parse().map_err(|e: String| anyhow::anyhow!(e))?;
            config.prover.apply_routes_override(&routes);
        }
        if let Some(base_url) = &cli.remote_sgx_base_url {
            config.prover.sgx.base_url.clone_from(base_url);
        }
        if let Some(base_url) = &cli.remote_sgx_sgxgeth_base_url {
            config.prover.sgxgeth.base_url.clone_from(base_url);
        }
        if let Some(timeout_ms) = cli.remote_sgx_timeout_ms {
            config.prover.sgx.timeout_ms = timeout_ms;
            config.prover.sgxgeth.timeout_ms = timeout_ms;
        }

        if let Some(queue_workers) = cli.queue_workers {
            config.queue.workers = queue_workers;
        }
        if let Some(interval_ms) = cli.queue_maintenance_interval_ms {
            config.queue.maintenance_interval_ms = interval_ms;
        }

        // Validate configuration
        config.validate()?;

        Ok(config)
    }

    /// Load configuration from a TOML file.
    pub fn from_file(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {}", path.display()))?;
        toml::from_str(&content)
            .with_context(|| format!("Failed to parse config file: {}", path.display()))
    }

    /// Validate the entire configuration.
    pub fn validate(&self) -> Result<()> {
        self.server
            .validate()
            .context("Server configuration error")?;
        let resolved_pairs = self
            .rpc
            .validate_base()
            .context("RPC configuration error")?;
        self.prover
            .validate()
            .context("Prover configuration error")?;
        self.runtime
            .validate()
            .context("Runtime configuration error")?;
        self.queue.validate().context("Queue configuration error")?;

        if self.prover.runner(ProofType::Risc0) == Some(RunnerKind::Network) {
            rpc::validate_boundless_pairs(&resolved_pairs)
                .context("Boundless RPC pair configuration error")?;
            for pair in &resolved_pairs {
                self.prover
                    .risc0
                    .boundless
                    .apply_pair_override(&pair.boundless)
                    .with_context(|| {
                        format!("Boundless configuration error for rpc pair {}", pair.key)
                    })?;
            }
        }
        if self.prover.is_enabled(ProofType::Sp1) {
            rpc::validate_sp1_verifier_pairs(&resolved_pairs)
                .context("SP1 verifier RPC pair configuration error")?;
        }

        self.preflight
            .validate(&resolved_pairs)
            .context("Preflight configuration error")?;
        Ok(())
    }
}

fn override_single_rpc_pair(
    rpc_config: &mut RpcConfig,
    update: impl FnOnce(&mut NetworkPairConfig),
) -> Result<()> {
    let [pair] = rpc_config.pairs.as_mut_slice() else {
        anyhow::bail!("RPC CLI endpoint overrides require exactly one rpc.pairs entry");
    };
    update(pair);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Cli, lock_test_cli_environment};
    use clap::Parser;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn write_temp_config(contents: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time went backwards")
            .as_nanos();
        path.push(format!("raiko2-config-{nanos}.toml"));
        std::fs::write(&path, contents).expect("write temp config");
        path
    }

    fn parse_config_cli(path: &Path) -> Cli {
        let _env_lock = lock_test_cli_environment();
        Cli::parse_from(["raiko2", "--config", path.to_str().expect("path utf8")])
    }

    fn workspace_config(path: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(path)
    }

    fn set_routes(config: &mut Config, routes: &str) {
        config
            .prover
            .apply_routes_override(&routes.parse().expect("valid prover routes"));
    }

    fn route_override_config() -> &'static str {
        r#"
[server]
host = "127.0.0.1"
port = 9090

[rpc]
pairs = [
  { network = "taiko_hoodi", l1_network = "hoodi", l1_rpc = "http://localhost:8545", l2_rpc = "http://localhost:9545" },
]

[prover.risc0]
enabled = true
runner = "local"

[prover.sp1]
enabled = true
prover = "local"

[prover.sgx]
base_url = "http://localhost:8080"
timeout_ms = 300000

[runtime]
environment = "test"
namespace = "raiko2-test"

[runtime.store]
backend = "memory"
"#
    }

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<String>,
    }

    impl EnvVarGuard {
        #[allow(unsafe_code)]
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var(key).ok();
            // SAFETY: callers hold the shared test CLI environment lock.
            unsafe { std::env::set_var(key, value) };
            Self { key, previous }
        }

        #[allow(unsafe_code)]
        fn remove(key: &'static str) -> Self {
            let previous = std::env::var(key).ok();
            // SAFETY: callers hold the shared test CLI environment lock.
            unsafe { std::env::remove_var(key) };
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        #[allow(unsafe_code)]
        fn drop(&mut self) {
            if let Some(previous) = &self.previous {
                // SAFETY: the guard is dropped before the shared environment lock.
                unsafe { std::env::set_var(self.key, previous) };
            } else {
                // SAFETY: the guard is dropped before the shared environment lock.
                unsafe { std::env::remove_var(self.key) };
            }
        }
    }

    #[test]
    fn test_server_config_default() {
        let config = ServerConfig::default();
        assert_eq!(config.host, "0.0.0.0");
        assert_eq!(config.port, 8080);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_config_example_validates() {
        let path = workspace_config("config.example.toml");
        let config = Config::from_file(&path).expect("parse config.example.toml");
        config.validate().expect("validate config.example.toml");
    }

    #[test]
    fn test_docker_compose_config_validates() {
        let path = workspace_config("docker/config.compose.toml");
        let config = Config::from_file(&path).expect("parse docker config");
        config.validate().expect("validate docker config");
    }

    #[test]
    fn test_server_config_debug_redacts_api_keys() {
        let config = ServerConfig {
            host: "localhost".to_string(),
            port: 8080,
            acl: ServerAclConfig {
                keys: vec![ServerAclKey {
                    id: "ops-clear".to_string(),
                    key: "secret-clear-key".to_string(),
                    allow: vec![ServerAclFeature::ProverClear],
                    rate_limit_per_minute: None,
                }],
            },
        };

        let debug = format!("{config:?}");
        assert!(!debug.contains("secret-clear-key"));
        assert!(debug.contains("<redacted>"));
    }

    #[test]
    fn test_server_config_invalid_clear_api_key() {
        let config = ServerConfig {
            host: "localhost".to_string(),
            port: 8080,
            acl: ServerAclConfig {
                keys: vec![ServerAclKey {
                    id: "ops-clear".to_string(),
                    key: String::new(),
                    allow: vec![ServerAclFeature::ProverClear],
                    rate_limit_per_minute: None,
                }],
            },
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_server_config_invalid_empty_acl_allow() {
        let config = ServerConfig {
            host: "localhost".to_string(),
            port: 8080,
            acl: ServerAclConfig {
                keys: vec![ServerAclKey {
                    id: "ops-clear".to_string(),
                    key: "secret-clear-key".to_string(),
                    allow: vec![],
                    rate_limit_per_minute: None,
                }],
            },
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_server_config_invalid_zero_acl_rate_limit() {
        let config = ServerConfig {
            host: "localhost".to_string(),
            port: 8080,
            acl: ServerAclConfig {
                keys: vec![ServerAclKey {
                    id: "submit".to_string(),
                    key: "secret-submit-key".to_string(),
                    allow: vec![ServerAclFeature::ProverSubmit],
                    rate_limit_per_minute: Some(0),
                }],
            },
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_server_config_rejects_duplicate_acl_ids() {
        let config = ServerConfig {
            host: "localhost".to_string(),
            port: 8080,
            acl: ServerAclConfig {
                keys: vec![
                    ServerAclKey {
                        id: "ops".to_string(),
                        key: "secret-clear-key".to_string(),
                        allow: vec![ServerAclFeature::ProverClear],
                        rate_limit_per_minute: None,
                    },
                    ServerAclKey {
                        id: "ops".to_string(),
                        key: "secret-submit-key".to_string(),
                        allow: vec![ServerAclFeature::ProverSubmit],
                        rate_limit_per_minute: None,
                    },
                ],
            },
        };
        let err = config.validate().expect_err("duplicate ACL id");
        assert!(err.to_string().contains("id must be unique"));
    }

    #[test]
    fn test_server_config_rejects_duplicate_acl_keys() {
        let config = ServerConfig {
            host: "localhost".to_string(),
            port: 8080,
            acl: ServerAclConfig {
                keys: vec![
                    ServerAclKey {
                        id: "ops-clear".to_string(),
                        key: "shared-secret-key".to_string(),
                        allow: vec![ServerAclFeature::ProverClear],
                        rate_limit_per_minute: None,
                    },
                    ServerAclKey {
                        id: "ops-submit".to_string(),
                        key: "shared-secret-key".to_string(),
                        allow: vec![ServerAclFeature::ProverSubmit],
                        rate_limit_per_minute: None,
                    },
                ],
            },
        };
        let err = config.validate().expect_err("duplicate ACL key");
        assert!(err.to_string().contains("key must be unique"));
    }

    #[test]
    fn test_server_config_invalid_host() {
        let config = ServerConfig {
            host: String::new(),
            port: 8080,
            acl: ServerAclConfig::default(),
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_server_config_invalid_port() {
        let config = ServerConfig {
            host: "localhost".to_string(),
            port: 0,
            acl: ServerAclConfig::default(),
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_rpc_config_default() {
        let config = RpcConfig::default();
        assert!(config.validate_base().is_ok());
    }

    #[test]
    fn test_rpc_config_valid_urls() {
        let config = RpcConfig {
            pairs: vec![NetworkPairConfig {
                network: "taiko_hoodi".to_string(),
                l1_network: "hoodi".to_string(),
                l1_rpc: Some("https://eth.llamarpc.com".to_string()),
                beacon_rpc: None,
                l2_rpc: Some("wss://taiko-rpc.example.com".to_string()),
                l2_provider: L2ProviderKind::Reth,
                l2_witness_rpc: Some("https://witness.taiko-rpc.example.com".to_string()),
                sp1_verifier_rpc_url: None,
                sp1_verifier_address: None,
                boundless: BoundlessPairConfig::default(),
            }],
            ..Default::default()
        };
        assert!(config.validate_base().is_ok());
    }

    #[test]
    fn test_rpc_config_invalid_l1_url() {
        let config = RpcConfig {
            pairs: vec![NetworkPairConfig {
                network: "taiko_hoodi".to_string(),
                l1_network: "hoodi".to_string(),
                l1_rpc: Some("not-a-valid-url".to_string()),
                beacon_rpc: None,
                l2_rpc: Some("http://localhost:9545".to_string()),
                l2_provider: L2ProviderKind::Reth,
                l2_witness_rpc: None,
                sp1_verifier_rpc_url: None,
                sp1_verifier_address: None,
                boundless: BoundlessPairConfig::default(),
            }],
            ..Default::default()
        };
        let result = config.validate_base();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("l1_rpc"));
    }

    #[test]
    fn test_rpc_config_rejects_partial_sp1_verifier_pair() {
        let config = RpcConfig {
            pairs: vec![NetworkPairConfig {
                network: "taiko_hoodi".to_string(),
                l1_network: "hoodi".to_string(),
                l1_rpc: Some("https://eth.llamarpc.com".to_string()),
                beacon_rpc: None,
                l2_rpc: Some("https://taiko-rpc.example.com".to_string()),
                l2_provider: L2ProviderKind::Reth,
                l2_witness_rpc: None,
                sp1_verifier_rpc_url: Some("https://verifier.example.com".to_string()),
                sp1_verifier_address: None,
                boundless: BoundlessPairConfig::default(),
            }],
            ..Default::default()
        };

        let pairs = config
            .validate_base()
            .expect("base RPC configuration should be valid");
        let result = rpc::validate_sp1_verifier_pairs(&pairs);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("taiko_hoodi/hoodi: sp1_verifier_address must be set")
        );
    }

    #[test]
    fn test_rpc_config_accepts_complete_sp1_verifier_pair() {
        let config = RpcConfig {
            pairs: vec![NetworkPairConfig {
                network: "taiko_hoodi".to_string(),
                l1_network: "hoodi".to_string(),
                l1_rpc: Some("https://eth.llamarpc.com".to_string()),
                beacon_rpc: None,
                l2_rpc: Some("https://taiko-rpc.example.com".to_string()),
                l2_provider: L2ProviderKind::Reth,
                l2_witness_rpc: None,
                sp1_verifier_rpc_url: Some("https://verifier.example.com".to_string()),
                sp1_verifier_address: Some(
                    "0x0000000000000000000000000000000000000001".to_string(),
                ),
                boundless: BoundlessPairConfig::default(),
            }],
            ..Default::default()
        };

        let pairs = config
            .validate_base()
            .expect("base RPC configuration should be valid");
        assert!(rpc::validate_sp1_verifier_pairs(&pairs).is_ok());
    }

    #[test]
    fn test_rpc_config_rejects_zero_sp1_verifier_address() {
        let config = RpcConfig {
            pairs: vec![NetworkPairConfig {
                network: "taiko_mainnet".to_string(),
                l1_network: "ethereum".to_string(),
                l1_rpc: Some("https://eth.llamarpc.com".to_string()),
                beacon_rpc: None,
                l2_rpc: Some("https://taiko-rpc.example.com".to_string()),
                l2_provider: L2ProviderKind::Reth,
                l2_witness_rpc: None,
                sp1_verifier_rpc_url: Some("https://verifier.example.com".to_string()),
                sp1_verifier_address: Some(
                    "0x0000000000000000000000000000000000000000".to_string(),
                ),
                boundless: BoundlessPairConfig::default(),
            }],
            ..Default::default()
        };

        let pairs = config
            .validate_base()
            .expect("base RPC configuration should be valid");
        let result = rpc::validate_sp1_verifier_pairs(&pairs);
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains(
                "taiko_mainnet/ethereum: sp1_verifier_address must not be the zero address"
            )
        );
    }

    #[test]
    fn test_rpc_config_requires_pairs() {
        let config = RpcConfig {
            pairs: Vec::new(),
            ..Default::default()
        };
        assert!(config.validate_base().is_err());
    }

    #[test]
    fn config_validate_skips_pair_boundless_when_network_route_disabled() {
        let mut config = Config::default();
        set_routes(&mut config, "risc0/local");
        config.rpc.pairs[0].boundless.rebid_timeout_ms = Some(0);

        config
            .validate()
            .expect("disabled Boundless settings must not reject startup");
    }

    #[test]
    fn config_validate_rejects_pair_boundless_when_network_route_enabled() {
        let mut config = Config::default();
        set_routes(&mut config, "risc0/network");
        config.prover.risc0.boundless.rpc_url = "https://boundless.example.com".to_string();
        config.prover.risc0.boundless.signer_key = "configured-by-secret-store".to_string();
        config.rpc.pairs[0].boundless.rebid_timeout_ms = Some(0);

        let err = config
            .validate()
            .expect_err("enabled Boundless settings must be validated");
        assert!(
            err.chain()
                .any(|source| source.to_string().contains("rebid_timeout_ms"))
        );
    }

    #[test]
    fn config_validate_skips_pair_sp1_when_route_disabled() {
        let mut config = Config::default();
        set_routes(&mut config, "native/local");
        config.rpc.pairs[0].sp1_verifier_rpc_url = Some("not a URL".to_string());
        config.rpc.pairs[0].sp1_verifier_address =
            Some("0x0000000000000000000000000000000000000001".to_string());

        config
            .validate()
            .expect("disabled SP1 settings must not reject startup");
    }

    #[test]
    fn config_validate_rejects_pair_sp1_when_route_enabled() {
        let mut config = Config::default();
        set_routes(&mut config, "sp1/local");
        config.prover.sp1.prover = raiko2_prover::sp1_config::ProverMode::Local;
        config.rpc.pairs[0].sp1_verifier_rpc_url = Some("not a URL".to_string());
        config.rpc.pairs[0].sp1_verifier_address =
            Some("0x0000000000000000000000000000000000000001".to_string());

        let err = config
            .validate()
            .expect_err("enabled SP1 settings must be validated");
        assert!(
            err.chain()
                .any(|source| source.to_string().contains("sp1_verifier_rpc_url"))
        );
    }

    #[test]
    fn test_config_rejects_invalid_pair_specific_boundless_offer() {
        let mut config = Config::default();
        set_routes(&mut config, "risc0/network");
        config.prover.risc0.boundless.rpc_url = "https://boundless.example.com".to_string();
        config.prover.risc0.boundless.signer_key = "configured-by-secret-store".to_string();
        config.rpc.pairs[0].boundless.offer_params.batch =
            Some(raiko2_prover::boundless_config::BoundlessOfferParams {
                timeouts: raiko2_prover::boundless_config::TimeoutPolicy::PerMcycle {
                    lock_timeout_ms_per_mcycle: 100,
                    timeout_ms_per_mcycle: 100,
                    dynamic_pricing_timeout_modifier: None,
                },
                ..config.prover.risc0.boundless.offer_params.batch.clone()
            });

        let err = config.validate().expect_err("invalid pair offer config");
        assert!(err.chain().any(|source| {
            source
                .to_string()
                .contains("timeout_ms_per_mcycle must be greater than lock_timeout_ms_per_mcycle")
        }));
    }

    #[test]
    fn test_pipeline_route_from_str() {
        assert_eq!(
            "risc0/local".parse::<PipelineRoute>().unwrap(),
            PipelineRoute::new(GuestSystem::Risc0, RunnerKind::Local)
        );
        assert_eq!(
            "RISC0/NETWORK".parse::<PipelineRoute>().unwrap(),
            PipelineRoute::new(GuestSystem::Risc0, RunnerKind::Network)
        );
        assert_eq!(
            "sp1/local".parse::<PipelineRoute>().unwrap(),
            PipelineRoute::new(GuestSystem::Sp1, RunnerKind::Local)
        );
        assert_eq!(
            "sp1/network".parse::<PipelineRoute>().unwrap(),
            PipelineRoute::new(GuestSystem::Sp1, RunnerKind::Network)
        );
        assert_eq!(
            "native/local".parse::<PipelineRoute>().unwrap(),
            PipelineRoute::new(GuestSystem::Native, RunnerKind::Local)
        );
        assert!("risc0/boundless".parse::<PipelineRoute>().is_err());
        assert!(
            "shasta-risc0-boundless"
                .parse::<raiko2_pipeline::PipelineKey>()
                .is_err()
        );
        assert_eq!(
            "sgx/remote".parse::<PipelineRoute>().unwrap(),
            PipelineRoute::new(GuestSystem::Sgx, RunnerKind::Remote)
        );
        assert!("invalid".parse::<PipelineRoute>().is_err());
    }

    #[test]
    fn test_config_default_enables_native_prover() {
        let config = Config::default();
        assert_eq!(
            config.prover.runner(ProofType::Native),
            Some(RunnerKind::Local)
        );
        assert_eq!(config.prover.iter_routes().count(), 1);
        config.validate().expect("default config is valid");
    }

    #[test]
    fn configless_load_uses_default_native_route() {
        let _env_lock = lock_test_cli_environment();
        let _config_guard = EnvVarGuard::remove("RAIKO2_CONFIG");
        let _routes_guard = EnvVarGuard::remove("RAIKO2_PROVER_ROUTES");
        let _legacy_guard = EnvVarGuard::remove("RAIKO2_PROVER");
        let cli = Cli::parse_from(["raiko2"]);

        let config = Config::load(&cli).expect("configless load should use default routes");
        assert_eq!(
            config.prover.iter_routes().collect::<Vec<_>>(),
            vec![(ProofType::Native, RunnerKind::Local)]
        );
    }

    #[test]
    fn configless_load_rejects_legacy_prover_env_with_routes_override() {
        let _env_lock = lock_test_cli_environment();
        let _config_guard = EnvVarGuard::remove("RAIKO2_CONFIG");
        let _routes_guard = EnvVarGuard::remove("RAIKO2_PROVER_ROUTES");
        let _legacy_guard = EnvVarGuard::set("RAIKO2_PROVER", "sgx/remote");
        let cli = Cli::parse_from(["raiko2", "--prover-routes", "native/local"]);

        let err = Config::load(&cli).expect_err("legacy prover env must be rejected");
        assert!(
            err.to_string()
                .contains("RAIKO2_PROVER is no longer supported"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn configless_load_accepts_explicit_prover_routes() {
        let _env_lock = lock_test_cli_environment();
        let _config_guard = EnvVarGuard::remove("RAIKO2_CONFIG");
        let _routes_guard = EnvVarGuard::remove("RAIKO2_PROVER_ROUTES");
        let cli = Cli::parse_from(["raiko2", "--prover-routes", "native/local"]);

        let config = Config::load(&cli).expect("explicit configless route should load");
        assert_eq!(
            config.prover.runner(raiko2_primitives::ProofType::Native),
            Some(RunnerKind::Local)
        );
        assert_eq!(config.prover.iter_routes().count(), 1);
    }

    #[test]
    fn prover_routes_cli_override_replaces_file_enablement() {
        let _env_lock = lock_test_cli_environment();
        let path = write_temp_config(route_override_config());
        let cli = Cli::parse_from([
            "raiko2",
            "--config",
            path.to_str().expect("path utf8"),
            "--prover-routes",
            "native/local",
        ]);

        let config = Config::load(&cli).expect("config load");
        assert_eq!(
            config.prover.iter_routes().collect::<Vec<_>>(),
            vec![(raiko2_primitives::ProofType::Native, RunnerKind::Local)]
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn prover_routes_env_override_replaces_file_enablement() {
        let _env_lock = lock_test_cli_environment();
        let path = write_temp_config(route_override_config());
        let _routes_guard = EnvVarGuard::set("RAIKO2_PROVER_ROUTES", "sgx/remote");
        let cli = Cli::parse_from(["raiko2", "--config", path.to_str().expect("path utf8")]);

        let config = Config::load(&cli).expect("config load");
        assert_eq!(
            config.prover.iter_routes().collect::<Vec<_>>(),
            vec![(raiko2_primitives::ProofType::Sgx, RunnerKind::Remote)]
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn prover_routes_cli_override_rejects_duplicate_entries() {
        let _env_lock = lock_test_cli_environment();
        let cli = Cli::parse_from(["raiko2", "--prover-routes", "risc0/local,risc0/network"]);

        let err = Config::load(&cli).expect_err("duplicate route override must fail");
        assert!(err.to_string().contains("duplicate prover route"));
    }

    #[test]
    fn file_config_requires_explicit_runtime_environment() {
        let mut value = toml::Value::try_from(Config::default()).expect("serialize config");
        value
            .as_table_mut()
            .expect("config table")
            .remove("runtime");

        let error = value
            .try_into::<Config>()
            .expect_err("runtime section must be explicit in deployment config");
        assert!(error.to_string().contains("runtime"));
    }

    #[test]
    fn test_boundless_route_requires_signer_key() {
        let mut config = Config::default();
        set_routes(&mut config, "risc0/network");
        config.prover.risc0.boundless.signer_key.clear();

        let err = config.prover.validate().expect_err("missing signer key");
        assert!(err.to_string().contains("signer_key"));
    }

    #[test]
    fn test_boundless_route_requires_rpc_url() {
        let mut config = Config::default();
        set_routes(&mut config, "risc0/network");
        config.prover.risc0.boundless.rpc_url.clear();
        config.prover.risc0.boundless.signer_key = "dummy-test-signer-key".to_string();

        let err = config.prover.validate().expect_err("missing rpc url");
        assert!(err.to_string().contains("rpc_url"));
    }

    #[test]
    fn test_sgx_remote_route_requires_sgx_base_url() {
        let mut config = Config::default();
        set_routes(&mut config, "sgx/remote");

        let err = config
            .prover
            .validate()
            .expect_err("missing remote sgx url");
        assert!(err.to_string().contains("prover.sgx.base_url"));
    }

    #[test]
    fn test_sgxgeth_remote_route_requires_sgxgeth_base_url() {
        let mut config = Config::default();
        set_routes(&mut config, "sgxgeth/remote");

        let err = config
            .prover
            .validate()
            .expect_err("missing remote sgxgeth url");
        assert!(err.to_string().contains("prover.sgxgeth.base_url"));
    }

    #[test]
    fn test_sgx_remote_route_accepts_configured_remote_sgx() {
        let mut config = Config::default();
        set_routes(&mut config, "sgx/remote");
        config.prover.sgx.base_url = "http://127.0.0.1:8080".to_string();
        config.prover.sgx.timeout_ms = 30_000;

        assert!(config.prover.validate().is_ok());
    }

    #[test]
    fn test_sgx_remote_route_accepts_sgxgeth_only_config() {
        let mut config = Config::default();
        set_routes(&mut config, "sgxgeth/remote");
        config.prover.sgxgeth.base_url = "http://127.0.0.1:8090".to_string();
        config.prover.sgxgeth.timeout_ms = 30_000;

        assert!(config.prover.validate().is_ok());
    }

    #[test]
    fn test_sgx_remote_route_env_overrides_remote_sgx_config() {
        let _env_lock = lock_test_cli_environment();
        let config_toml = r#"
[server]
host = "0.0.0.0"
port = 8080

[rpc]
pairs = [
  { network = "taiko_mainnet", l1_network = "ethereum", l1_rpc = "http://localhost:8545", l2_rpc = "http://localhost:9545" },
]

[prover.sgx]
enabled = true
base_url = "http://127.0.0.1:8080"
timeout_ms = 300000

[prover.sgxgeth]
enabled = true
base_url = "http://127.0.0.1:8090"
timeout_ms = 300001

[queue]
workers = 1
maintenance_interval_ms = 200

[runtime]
environment = "test"
namespace = "raiko2-test"

[runtime.store]
backend = "memory"
"#;
        let path = write_temp_config(config_toml);
        let _base_url_guard =
            EnvVarGuard::set("RAIKO2_REMOTE_SGX_BASE_URL", "http://127.0.0.1:19090");
        let _sgxgeth_base_url_guard = EnvVarGuard::set(
            "RAIKO2_REMOTE_SGX_SGXGETH_BASE_URL",
            "http://127.0.0.1:19091",
        );
        let _timeout_guard = EnvVarGuard::set("RAIKO2_REMOTE_SGX_TIMEOUT_MS", "12345");

        let cli = Cli::parse_from(["raiko2", "--config", path.to_str().expect("path utf8")]);

        let config = Config::load(&cli).expect("config load");
        assert_eq!(config.prover.sgx.base_url, "http://127.0.0.1:19090");
        assert_eq!(config.prover.sgxgeth.base_url, "http://127.0.0.1:19091");
        assert_eq!(config.prover.sgx.timeout_ms, 12_345);
        assert_eq!(config.prover.sgxgeth.timeout_ms, 12_345);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_risc0_execution_po2_must_be_non_zero() {
        let mut config = Config::default();
        set_routes(&mut config, "risc0/local");
        config.prover.risc0.execution_po2 = 0;

        let err = config.prover.validate().expect_err("zero po2 should fail");
        assert!(err.to_string().contains("execution_po2"));
    }

    #[test]
    fn test_config_loads_risc0_execution_po2() {
        let config_toml = r#"
[server]
host = "0.0.0.0"
port = 8080

[rpc]
pairs = [
  { network = "taiko_hoodi", l1_network = "hoodi", l1_rpc = "http://localhost:8545", l2_rpc = "http://localhost:9545" },
]

[prover.risc0]
enabled = true
runner = "local"
execution_po2 = 24

[queue]
workers = 1
maintenance_interval_ms = 200

[runtime]
environment = "test"
namespace = "raiko2-test"

[runtime.store]
backend = "memory"
"#;
        let path = write_temp_config(config_toml);

        let cli = parse_config_cli(&path);

        let config = Config::load(&cli).expect("config load");
        assert_eq!(config.prover.risc0.execution_po2, 24);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_config_file_values_are_not_overridden_by_cli_defaults() {
        let config_toml = r#"
[server]
host = "127.0.0.1"
port = 9090

[rpc]
pairs = [
  { network = "taiko_hoodi", l1_network = "hoodi", l1_rpc = "https://hoodi.example.test", l2_rpc = "http://taiko-hoodi.example.test:8545" },
]

[prover.native]
enabled = true

[queue]
workers = 1
maintenance_interval_ms = 200

[runtime]
environment = "test"
namespace = "raiko2-test"

[runtime.store]
backend = "memory"
"#;
        let path = write_temp_config(config_toml);

        let cli = parse_config_cli(&path);

        let config = Config::load(&cli).expect("config load");
        assert_eq!(config.server.host, "127.0.0.1");
        assert_eq!(config.server.port, 9090);
        let pair = config
            .rpc
            .resolve_pair("taiko_hoodi", "hoodi")
            .expect("resolved pair");
        assert_eq!(pair.l1_chain_id(), 560_048);
        assert_eq!(pair.l2_chain_id(), 167_013);
        assert_eq!(
            config.prover.runner(raiko2_primitives::ProofType::Native),
            Some(RunnerKind::Local)
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_cli_sp1_network_route_updates_sp1_execution_selector() {
        let _env_lock = lock_test_cli_environment();
        let cli = Cli::parse_from(["raiko2", "--prover-routes", "sp1/network"]);

        let config = Config::load(&cli).expect("config load");
        assert_eq!(
            config.prover.runner(raiko2_primitives::ProofType::Sp1),
            Some(RunnerKind::Network)
        );
        assert_eq!(
            config.prover.sp1.prover,
            raiko2_prover::sp1_config::ProverMode::Network
        );
    }

    #[test]
    fn test_pairs_only_config_loads_without_legacy_rpc_fields() {
        let config_toml = r#"
[server]
host = "127.0.0.1"
port = 9090

[rpc]
pairs = [
  { network = "taiko_hoodi", l1_network = "hoodi", l1_rpc = "http://hoodi.example.test:8545", l2_rpc = "http://taiko-hoodi.example.test:8545" },
]

[rpc.client]
concurrency_limit = 24

[prover.native]
enabled = true

[queue]
workers = 1
maintenance_interval_ms = 200

[runtime]
environment = "test"
namespace = "raiko2-test"

[runtime.store]
backend = "memory"
"#;
        let path = write_temp_config(config_toml);

        let cli = parse_config_cli(&path);

        let config = Config::load(&cli).expect("config load");
        let pair = config
            .rpc
            .resolve_pair("taiko_hoodi", "hoodi")
            .expect("resolved pair");
        assert_eq!(pair.l1_rpc, "http://hoodi.example.test:8545");
        assert_eq!(pair.l2_rpc, "http://taiko-hoodi.example.test:8545");
        assert_eq!(pair.l2_provider, L2ProviderKind::Reth);
        assert_eq!(pair.l2_witness_rpc, "http://taiko-hoodi.example.test:8545");
        assert_eq!(pair.l1_chain_id(), 560_048);
        assert_eq!(pair.l2_chain_id(), 167_013);
        assert_eq!(config.rpc.client.concurrency_limit, 24);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_l2_rpc_cli_override_preserves_configured_witness_rpc() {
        let config_toml = r#"
[server]
host = "127.0.0.1"
port = 9090

[rpc]
pairs = [
  { network = "taiko_hoodi", l1_network = "hoodi", l1_rpc = "http://localhost:8545", l2_rpc = "http://localhost:9545", l2_witness_rpc = "http://localhost:9547" },
]

[prover.native]
enabled = true

[runtime]
environment = "test"
namespace = "raiko2-test"

[runtime.store]
backend = "memory"
"#;
        let path = write_temp_config(config_toml);
        let _env_lock = lock_test_cli_environment();
        let cli = Cli::parse_from([
            "raiko2",
            "--config",
            path.to_str().expect("path utf8"),
            "--l2-rpc",
            "http://localhost:9555",
        ]);

        let config = Config::load(&cli).expect("config load");
        let pair = config
            .rpc
            .resolve_pair("taiko_hoodi", "hoodi")
            .expect("resolved pair");
        assert_eq!(pair.l2_rpc, "http://localhost:9555");
        assert_eq!(pair.l2_witness_rpc, "http://localhost:9547");

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_config_rejects_unknown_fields() {
        let config_toml = r#"
[server]
host = "127.0.0.1"
port = 9090
unexpected = true

[rpc]
pairs = [
  { network = "taiko_hoodi", l1_network = "hoodi", l1_rpc = "http://localhost:8545", l2_rpc = "http://localhost:9545" },
]

[prover.native]
enabled = true

[runtime]
environment = "test"
namespace = "raiko2-test"

[runtime.store]
backend = "memory"
"#;
        let path = write_temp_config(config_toml);
        let cli = parse_config_cli(&path);

        let err = Config::load(&cli).expect_err("unknown config field must fail");
        assert!(
            err.chain()
                .any(|source| source.to_string().contains("unknown field")),
            "unexpected error: {err}"
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_config_rejects_legacy_queue_retry_table() {
        let config_toml = r#"
[server]
host = "127.0.0.1"
port = 9090

[rpc]
pairs = [
  { network = "taiko_hoodi", l1_network = "hoodi", l1_rpc = "http://localhost:8545", l2_rpc = "http://localhost:9545" },
]

[prover.native]
enabled = true

[queue]

[queue.retry]
strategy = "fixed"

[runtime]
environment = "test"
namespace = "raiko2-test"

[runtime.store]
backend = "memory"
"#;
        let path = write_temp_config(config_toml);
        let cli = parse_config_cli(&path);

        let err = Config::load(&cli).expect_err("legacy queue retry config must fail");
        assert!(
            err.chain()
                .any(|source| source.to_string().contains("unknown field")),
            "unexpected error: {err}"
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_config_file_accepts_geth_l2_provider() {
        let config_toml = r#"
[server]
host = "0.0.0.0"
port = 8080

[rpc]
pairs = [
  { network = "taiko_hoodi", l1_network = "hoodi", l1_rpc = "http://hoodi.example.test:8545", l2_rpc = "http://taiko-hoodi.example.test:8545", l2_provider = "geth" },
]

[prover.native]
enabled = true

[queue]
workers = 1
maintenance_interval_ms = 200

[runtime]
environment = "test"
namespace = "raiko2-test"

[runtime.store]
backend = "memory"
"#;
        let path = write_temp_config(config_toml);

        let cli = parse_config_cli(&path);
        let config = Config::load(&cli).expect("config load");
        let pair = config
            .rpc
            .resolve_pair("taiko_hoodi", "hoodi")
            .expect("resolved pair");

        assert_eq!(pair.l2_provider, L2ProviderKind::Geth);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_config_file_accepts_geth_local_witness_l2_provider() {
        let config_toml = r#"
[server]
host = "0.0.0.0"
port = 8080

[rpc]
pairs = [
  { network = "taiko_hoodi", l1_network = "hoodi", l1_rpc = "https://ethereum-hoodi-rpc.publicnode.com", l2_rpc = "https://rpc.hoodi.taiko.xyz", l2_provider = "geth_local_witness" },
]

[prover.native]
enabled = true

[queue]
workers = 1
maintenance_interval_ms = 200

[runtime]
environment = "test"
namespace = "raiko2-test"

[runtime.store]
backend = "memory"
"#;
        let path = write_temp_config(config_toml);

        let cli = parse_config_cli(&path);
        let config = Config::load(&cli).expect("config load");
        let pair = config
            .rpc
            .resolve_pair("taiko_hoodi", "hoodi")
            .expect("resolved pair");

        assert_eq!(pair.l2_provider, L2ProviderKind::GethLocalWitness);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_config_file_accepts_preflight_verify_checkpoint_l2_rpc_map() {
        let config_toml = r#"
[server]
host = "0.0.0.0"
port = 8080

[rpc]
pairs = [
  { network = "taiko_hoodi", l1_network = "hoodi", l1_rpc = "https://ethereum-hoodi-rpc.publicnode.com", l2_rpc = "https://rpc.hoodi.taiko.xyz" },
]

[preflight.verify_checkpoint_l2_rpcs]
taiko_hoodi = "https://verify.hoodi.example"

[prover.native]
enabled = true

[runtime]
environment = "test"
namespace = "raiko2-test"

[runtime.store]
backend = "memory"
"#;
        let path = write_temp_config(config_toml);
        let cli = parse_config_cli(&path);

        let config = Config::load(&cli).expect("config load");
        assert_eq!(
            config
                .preflight
                .verify_checkpoint_l2_rpcs
                .get("taiko_hoodi")
                .map(String::as_str),
            Some("https://verify.hoodi.example")
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_config_rejects_preflight_verify_checkpoint_rpc_for_ambiguous_network() {
        let config_toml = r#"
[server]
host = "0.0.0.0"
port = 8080

[rpc]
pairs = [
  { network = "taiko_dev", l1_network = "hoodi", l1_rpc = "https://ethereum-hoodi-rpc.publicnode.com", l2_rpc = "https://rpc.hoodi.taiko.xyz" },
  { network = "taiko_dev", l1_network = "ethereum", l1_rpc = "https://ethereum-rpc.publicnode.com", l2_rpc = "https://rpc.mainnet.taiko.xyz" },
]

[preflight.verify_checkpoint_l2_rpcs]
taiko_dev = "https://verify.dev.example"

[prover.native]
enabled = true

[runtime]
environment = "test"
namespace = "raiko2-test"

[runtime.store]
backend = "memory"
"#;
        let path = write_temp_config(config_toml);
        let cli = parse_config_cli(&path);

        let err = Config::load(&cli).expect_err("ambiguous network verify rpc must fail");
        let err_text = format!("{err:#}");
        assert!(
            err_text.contains("ambiguous") && err_text.contains("taiko_dev"),
            "unexpected error: {err_text}"
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_is_valid_url() {
        assert!(super::rpc::is_valid_url("http://localhost:8545"));
        assert!(super::rpc::is_valid_url("https://eth.llamarpc.com"));
        assert!(super::rpc::is_valid_url("ws://localhost:8546"));
        assert!(super::rpc::is_valid_url("wss://rpc.example.com"));
        assert!(!super::rpc::is_valid_url("localhost:8545"));
        assert!(!super::rpc::is_valid_url("ftp://files.example.com"));
        assert!(!super::rpc::is_valid_url("http://"));
        assert!(!super::rpc::is_valid_url("https://"));
        assert!(!super::rpc::is_valid_url("http:///"));
        assert!(!super::rpc::is_valid_url("http://localhost:bad-port"));
        assert!(!super::rpc::is_valid_url(""));
    }
}
