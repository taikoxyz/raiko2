use alloy_primitives::Address;
use anyhow::{Result, bail};
use raiko2_primitives::{ChainSpec, SupportedChainSpecs};
use raiko2_provider::{
    RpcClientConfig as ProviderRpcClientConfig, RpcRetryConfig as ProviderRpcRetryConfig,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::str::FromStr;

const fn default_rpc_timeout_ms() -> u64 {
    60_000
}

const fn default_rpc_concurrency_limit() -> usize {
    32
}

const fn default_rpc_retry_max_attempts() -> u32 {
    4
}

const fn default_rpc_retry_initial_backoff_ms() -> u64 {
    1_000
}

const fn default_rpc_retry_cu_per_second() -> u64 {
    1_000
}

use super::validation;

fn default_rpc_pairs() -> Vec<NetworkPairConfig> {
    vec![NetworkPairConfig {
        network: "taiko_hoodi".to_string(),
        l1_network: "hoodi".to_string(),
        l1_rpc: None,
        l2_rpc: None,
        l2_witness_rpc: None,
        sp1_verifier_rpc_url: None,
        sp1_verifier_address: None,
    }]
}

/// Validate that a string is a valid URL.
pub(crate) fn is_valid_url(url: &str) -> bool {
    url.starts_with("http://")
        || url.starts_with("https://")
        || url.starts_with("ws://")
        || url.starts_with("wss://")
}

/// Explicitly allowed L2/L1 network pair.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NetworkPairConfig {
    pub network: String,
    pub l1_network: String,
    #[serde(default)]
    pub l1_rpc: Option<String>,
    #[serde(default)]
    pub l2_rpc: Option<String>,
    #[serde(default)]
    pub l2_witness_rpc: Option<String>,
    #[serde(default)]
    pub sp1_verifier_rpc_url: Option<String>,
    #[serde(default)]
    pub sp1_verifier_address: Option<String>,
}

impl NetworkPairConfig {
    #[must_use]
    pub fn key(&self) -> String {
        format!("{}/{}", self.network, self.l1_network)
    }
}

/// Resolved L2/L1 network pair with built-in chain specs attached.
#[derive(Debug, Clone)]
pub struct ResolvedNetworkPair {
    pub key: String,
    pub network: String,
    pub l1_network: String,
    pub l1_rpc: String,
    pub l2_rpc: String,
    pub l2_witness_rpc: String,
    pub sp1_verifier_rpc_url: Option<String>,
    pub sp1_verifier_address: Option<String>,
    pub l1_spec: ChainSpec,
    pub l2_spec: ChainSpec,
}

impl ResolvedNetworkPair {
    #[must_use]
    pub const fn l1_chain_id(&self) -> u64 {
        self.l1_spec.chain_id()
    }

    #[must_use]
    pub const fn l2_chain_id(&self) -> u64 {
        self.l2_spec.chain_id()
    }
}

/// RPC configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RpcConfig {
    #[serde(default = "default_rpc_pairs")]
    pub pairs: Vec<NetworkPairConfig>,
    #[serde(default)]
    pub client: RpcClientConfig,
}

impl Default for RpcConfig {
    fn default() -> Self {
        Self {
            pairs: default_rpc_pairs(),
            client: RpcClientConfig::default(),
        }
    }
}

/// RPC client configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcClientConfig {
    #[serde(default = "default_rpc_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default = "default_rpc_concurrency_limit")]
    pub concurrency_limit: usize,
    #[serde(default)]
    pub retry: RpcRetryConfig,
}

impl Default for RpcClientConfig {
    fn default() -> Self {
        Self {
            timeout_ms: default_rpc_timeout_ms(),
            concurrency_limit: default_rpc_concurrency_limit(),
            retry: RpcRetryConfig::default(),
        }
    }
}

/// RPC retry configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcRetryConfig {
    #[serde(default = "default_rpc_retry_max_attempts")]
    pub max_attempts: u32,
    #[serde(default = "default_rpc_retry_initial_backoff_ms")]
    pub initial_backoff_ms: u64,
    #[serde(default = "default_rpc_retry_cu_per_second")]
    pub compute_units_per_second: u64,
}

impl Default for RpcRetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: default_rpc_retry_max_attempts(),
            initial_backoff_ms: default_rpc_retry_initial_backoff_ms(),
            compute_units_per_second: default_rpc_retry_cu_per_second(),
        }
    }
}

impl RpcConfig {
    /// Validate RPC configuration.
    pub fn validate(&self) -> Result<()> {
        if self.client.timeout_ms == 0 {
            bail!("rpc client timeout_ms must be > 0");
        }
        if self.client.concurrency_limit == 0 {
            bail!("rpc client concurrency_limit must be > 0");
        }
        if self.client.retry.max_attempts != 0 {
            if self.client.retry.initial_backoff_ms == 0 {
                bail!("rpc client retry initial_backoff_ms must be > 0");
            }
            if self.client.retry.compute_units_per_second == 0 {
                bail!("rpc client retry compute_units_per_second must be > 0");
            }
        }

        let mut seen = HashSet::new();
        for pair in self.resolved_pairs()? {
            if !seen.insert(pair.key.clone()) {
                bail!("duplicate rpc pair configuration: {}", pair.key);
            }
            if !is_valid_url(&pair.l1_rpc) {
                bail!(
                    "{}: l1_rpc = '{}'",
                    validation::INVALID_RPC_URL,
                    pair.l1_rpc
                );
            }
            if !is_valid_url(&pair.l2_rpc) {
                bail!(
                    "{}: l2_rpc = '{}'",
                    validation::INVALID_RPC_URL,
                    pair.l2_rpc
                );
            }
            if !is_valid_url(&pair.l2_witness_rpc) {
                bail!(
                    "{}: l2_witness_rpc = '{}'",
                    validation::INVALID_RPC_URL,
                    pair.l2_witness_rpc
                );
            }
            match (&pair.sp1_verifier_rpc_url, &pair.sp1_verifier_address) {
                (Some(rpc_url), Some(address)) => {
                    if !is_valid_url(rpc_url) {
                        bail!(
                            "{}: sp1_verifier_rpc_url = '{}'",
                            validation::INVALID_RPC_URL,
                            rpc_url
                        );
                    }
                    if Address::from_str(address).is_err() {
                        bail!("invalid sp1_verifier_address = '{address}'");
                    }
                }
                (None, None) => {}
                (Some(_), None) => {
                    bail!("sp1_verifier_address must be set when sp1_verifier_rpc_url is set");
                }
                (None, Some(_)) => {
                    bail!("sp1_verifier_rpc_url must be set when sp1_verifier_address is set");
                }
            }
            if !pair.l2_spec.is_taiko() {
                bail!("rpc pair {} must target a Taiko L2 network", pair.key);
            }
        }
        Ok(())
    }

    /// Resolve the configured RPC matrix into explicit network pairs.
    pub fn resolved_pairs(&self) -> Result<Vec<ResolvedNetworkPair>> {
        if self.pairs.is_empty() {
            bail!("rpc.pairs must contain at least one network pair");
        }
        let known_specs = SupportedChainSpecs::default();
        self.pairs
            .iter()
            .map(|pair| resolve_pair(&known_specs, pair))
            .collect()
    }

    /// Resolve a single allowed pair by public network names.
    pub fn resolve_pair(&self, network: &str, l1_network: &str) -> Result<ResolvedNetworkPair> {
        self.resolved_pairs()?
            .into_iter()
            .find(|pair| pair.network == network && pair.l1_network == l1_network)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "unsupported network pair: network={network}, l1_network={l1_network}"
                )
            })
    }

    #[must_use]
    pub const fn provider_client_config(&self) -> ProviderRpcClientConfig {
        ProviderRpcClientConfig {
            timeout_ms: self.client.timeout_ms,
            concurrency_limit: self.client.concurrency_limit,
            retry: ProviderRpcRetryConfig {
                max_attempts: self.client.retry.max_attempts,
                initial_backoff_ms: self.client.retry.initial_backoff_ms,
                compute_units_per_second: self.client.retry.compute_units_per_second,
            },
        }
    }
}

fn resolve_pair(
    known_specs: &SupportedChainSpecs,
    pair: &NetworkPairConfig,
) -> Result<ResolvedNetworkPair> {
    let l2_spec = known_specs
        .get_chain_spec(&pair.network)
        .ok_or_else(|| anyhow::anyhow!("unsupported L2 network '{}'", pair.network))?;
    let l1_spec = known_specs
        .get_chain_spec(&pair.l1_network)
        .ok_or_else(|| anyhow::anyhow!("unsupported L1 network '{}'", pair.l1_network))?;
    let l2_rpc = pair.l2_rpc.clone().unwrap_or_else(|| l2_spec.rpc.clone());

    Ok(ResolvedNetworkPair {
        key: pair.key(),
        network: pair.network.clone(),
        l1_network: pair.l1_network.clone(),
        l1_rpc: pair.l1_rpc.clone().unwrap_or_else(|| l1_spec.rpc.clone()),
        l2_rpc: l2_rpc.clone(),
        l2_witness_rpc: pair.l2_witness_rpc.clone().unwrap_or(l2_rpc),
        sp1_verifier_rpc_url: pair.sp1_verifier_rpc_url.clone(),
        sp1_verifier_address: pair.sp1_verifier_address.clone(),
        l1_spec,
        l2_spec,
    })
}
