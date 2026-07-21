use alloy_primitives::Address;
use anyhow::{Context, Result, bail};
use raiko2_primitives::{ChainSpec, SupportedChainSpecs};
use raiko2_prover::boundless_config::{
    BoundlessOfferParams, MIN_REBID_TIMEOUT_MS, QuoteSizing, REBID_MAX_ATTEMPTS_LIMIT,
    validate_offer_spec,
};
use raiko2_provider::{
    DEFAULT_RPC_TIMEOUT_MS, L2ProviderKind, RpcClientConfig as ProviderRpcClientConfig,
    RpcRetryConfig as ProviderRpcRetryConfig,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::str::FromStr;
use url::Url;

const fn default_rpc_timeout_ms() -> u64 {
    DEFAULT_RPC_TIMEOUT_MS
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
        beacon_rpc: None,
        l2_rpc: None,
        l2_provider: L2ProviderKind::default(),
        l2_witness_rpc: None,
        sp1_verifier_rpc_url: None,
        sp1_verifier_address: None,
        boundless: BoundlessPairConfig::default(),
    }]
}

/// Validate that a string is a valid URL.
pub(crate) fn is_valid_url(url: &str) -> bool {
    let Ok(url) = Url::parse(url) else {
        return false;
    };
    matches!(url.scheme(), "http" | "https" | "ws" | "wss") && url.host_str().is_some()
}

/// Explicitly allowed L2/L1 network pair.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NetworkPairConfig {
    pub network: String,
    pub l1_network: String,
    #[serde(default)]
    pub l1_rpc: Option<String>,
    #[serde(default)]
    pub beacon_rpc: Option<String>,
    #[serde(default)]
    pub l2_rpc: Option<String>,
    #[serde(default)]
    pub l2_provider: L2ProviderKind,
    #[serde(default)]
    pub l2_witness_rpc: Option<String>,
    #[serde(default)]
    pub sp1_verifier_rpc_url: Option<String>,
    #[serde(default)]
    pub sp1_verifier_address: Option<String>,
    #[serde(default)]
    pub boundless: BoundlessPairConfig,
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
    pub l2_provider: L2ProviderKind,
    pub l2_witness_rpc: String,
    pub sp1_verifier_rpc_url: Option<String>,
    pub sp1_verifier_address: Option<String>,
    pub boundless: BoundlessPairConfig,
    pub l1_spec: ChainSpec,
    pub l2_spec: ChainSpec,
}

/// Pair-specific Boundless overrides for RISC0 network routes.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct BoundlessPairConfig {
    pub batch_quote: Option<QuoteSizing>,
    pub aggregation_quote: Option<QuoteSizing>,
    pub poll_interval_ms: Option<u64>,
    pub timeout_ms: Option<u64>,
    pub rebid_timeout_ms: Option<u64>,
    pub rebid_price_step_bps: Option<u32>,
    pub rebid_max_attempts: Option<u32>,
    pub offer_params: BoundlessOfferParamsOverride,
}

impl BoundlessPairConfig {
    /// Validate the optional pair-specific Boundless overrides.
    pub fn validate(&self, pair_key: &str) -> Result<()> {
        if let Some(q) = &self.batch_quote {
            q.validate(&format!("{pair_key}: boundless.batch_quote"))
                .map_err(anyhow::Error::msg)?;
        }
        if let Some(q) = &self.aggregation_quote {
            q.validate(&format!("{pair_key}: boundless.aggregation_quote"))
                .map_err(anyhow::Error::msg)?;
        }
        if matches!(self.poll_interval_ms, Some(0)) {
            bail!("{pair_key}: boundless.poll_interval_ms must be > 0");
        }
        if matches!(self.timeout_ms, Some(0)) {
            bail!("{pair_key}: boundless.timeout_ms must be > 0");
        }
        if matches!(self.rebid_timeout_ms, Some(value) if value < MIN_REBID_TIMEOUT_MS) {
            bail!("{pair_key}: boundless.rebid_timeout_ms must be >= {MIN_REBID_TIMEOUT_MS}");
        }
        if let Some(rebid_max_attempts) = self.rebid_max_attempts
            && rebid_max_attempts > REBID_MAX_ATTEMPTS_LIMIT
        {
            bail!("{pair_key}: boundless.rebid_max_attempts must be <= {REBID_MAX_ATTEMPTS_LIMIT}");
        }
        if let Some(batch) = &self.offer_params.batch {
            validate_offer_spec(batch)
                .map_err(anyhow::Error::msg)
                .map_err(|err| {
                    anyhow::anyhow!("{pair_key}: boundless.offer_params.batch: {err}")
                })?;
        }
        if let Some(aggregation) = &self.offer_params.aggregation {
            validate_offer_spec(aggregation)
                .map_err(anyhow::Error::msg)
                .map_err(|err| {
                    anyhow::anyhow!("{pair_key}: boundless.offer_params.aggregation: {err}")
                })?;
        }
        Ok(())
    }
}

/// Optional pair-specific overrides for the Boundless batch and aggregation offer params.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct BoundlessOfferParamsOverride {
    pub batch: Option<BoundlessOfferParams>,
    pub aggregation: Option<BoundlessOfferParams>,
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
#[serde(default, deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
        let pairs = self.validate_base()?;
        validate_boundless_pairs(&pairs)?;
        validate_sp1_verifier_pairs(&pairs)?;
        Ok(())
    }

    pub(super) fn validate_base(&self) -> Result<Vec<ResolvedNetworkPair>> {
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
        let pairs = self.resolved_pairs()?;
        for pair in &pairs {
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
            if let Some(beacon_rpc) = &pair.l1_spec.beacon_rpc
                && !is_valid_url(beacon_rpc)
            {
                bail!(
                    "{}: beacon_rpc = '{}'",
                    validation::INVALID_RPC_URL,
                    beacon_rpc
                );
            }
            if pair.l1_spec.seconds_per_slot == 0 {
                bail!("{}: L1 chain spec seconds_per_slot must be > 0", pair.key);
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
            if !pair.l2_spec.is_taiko() {
                bail!("rpc pair {} must target a Taiko L2 network", pair.key);
            }
        }
        Ok(pairs)
    }

    /// Resolve the configured RPC matrix into explicit network pairs.
    pub fn resolved_pairs(&self) -> Result<Vec<ResolvedNetworkPair>> {
        if self.pairs.is_empty() {
            bail!("rpc.pairs must contain at least one network pair");
        }
        let known_specs = SupportedChainSpecs::default();
        known_specs
            .validate_host_sanity()
            .context("default chain spec sanity check failed")?;
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

pub(super) fn validate_boundless_pairs(pairs: &[ResolvedNetworkPair]) -> Result<()> {
    for pair in pairs {
        pair.boundless.validate(&pair.key)?;
    }
    Ok(())
}

pub(super) fn validate_sp1_verifier_pairs(pairs: &[ResolvedNetworkPair]) -> Result<()> {
    for pair in pairs {
        match (&pair.sp1_verifier_rpc_url, &pair.sp1_verifier_address) {
            (Some(rpc_url), Some(address)) => {
                if !is_valid_url(rpc_url) {
                    bail!(
                        "{}: sp1_verifier_rpc_url for {}",
                        validation::INVALID_RPC_URL,
                        pair.key
                    );
                }
                let verifier_address = Address::from_str(address).map_err(|_| {
                    anyhow::anyhow!("{}: invalid sp1_verifier_address = '{address}'", pair.key)
                })?;
                if verifier_address == Address::ZERO {
                    bail!(
                        "{}: sp1_verifier_address must not be the zero address",
                        pair.key
                    );
                }
            }
            (None, None) => {}
            (Some(_), None) => {
                bail!(
                    "{}: sp1_verifier_address must be set when sp1_verifier_rpc_url is set",
                    pair.key
                );
            }
            (None, Some(_)) => {
                bail!(
                    "{}: sp1_verifier_rpc_url must be set when sp1_verifier_address is set",
                    pair.key
                );
            }
        }
    }
    Ok(())
}

fn resolve_pair(
    known_specs: &SupportedChainSpecs,
    pair: &NetworkPairConfig,
) -> Result<ResolvedNetworkPair> {
    let l2_spec = known_specs
        .get_chain_spec(&pair.network)
        .ok_or_else(|| anyhow::anyhow!("unsupported L2 network '{}'", pair.network))?;
    let mut l1_spec = known_specs
        .get_chain_spec(&pair.l1_network)
        .ok_or_else(|| anyhow::anyhow!("unsupported L1 network '{}'", pair.l1_network))?;
    if let Some(beacon_rpc) = &pair.beacon_rpc {
        l1_spec.beacon_rpc = Some(beacon_rpc.clone());
    }
    let l2_rpc = pair.l2_rpc.clone().unwrap_or_else(|| l2_spec.rpc.clone());

    Ok(ResolvedNetworkPair {
        key: pair.key(),
        network: pair.network.clone(),
        l1_network: pair.l1_network.clone(),
        l1_rpc: pair.l1_rpc.clone().unwrap_or_else(|| l1_spec.rpc.clone()),
        l2_rpc: l2_rpc.clone(),
        l2_provider: pair.l2_provider,
        l2_witness_rpc: pair.l2_witness_rpc.clone().unwrap_or(l2_rpc),
        sp1_verifier_rpc_url: pair.sp1_verifier_rpc_url.clone(),
        sp1_verifier_address: pair.sp1_verifier_address.clone(),
        boundless: pair.boundless.clone(),
        l1_spec,
        l2_spec,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rpc_config_default_timeout_matches_provider_default() {
        let config = RpcConfig::default();

        assert_eq!(config.client.timeout_ms, DEFAULT_RPC_TIMEOUT_MS);
        assert_eq!(
            config.provider_client_config().timeout_ms,
            DEFAULT_RPC_TIMEOUT_MS
        );
    }

    #[test]
    fn boundless_pair_config_rejects_zero_aggregation_quote() {
        let config = BoundlessPairConfig {
            aggregation_quote: Some(QuoteSizing::Fixed { mcycles: 0 }),
            ..Default::default()
        };

        assert!(
            config
                .validate("taiko_hoodi/hoodi")
                .expect_err("zero aggregation quote should fail")
                .to_string()
                .contains("aggregation_quote")
        );
    }

    #[test]
    fn boundless_pair_config_rejects_zero_rebid_timeout() {
        let config = BoundlessPairConfig {
            rebid_timeout_ms: Some(0),
            ..Default::default()
        };

        assert!(
            config
                .validate("taiko_hoodi/hoodi")
                .expect_err("zero rebid timeout should fail")
                .to_string()
                .contains("rebid_timeout_ms")
        );
    }

    #[test]
    fn boundless_pair_config_rejects_subsecond_rebid_timeout() {
        let config = BoundlessPairConfig {
            rebid_timeout_ms: Some(999),
            ..Default::default()
        };

        assert!(
            config
                .validate("taiko_hoodi/hoodi")
                .expect_err("subsecond rebid timeout should fail")
                .to_string()
                .contains("rebid_timeout_ms")
        );
    }

    #[test]
    fn boundless_pair_config_accepts_zero_rebid_price_step_bps() {
        let config = BoundlessPairConfig {
            rebid_price_step_bps: Some(0),
            ..Default::default()
        };

        // 0 bps is a valid flat (no-escalation) ladder.
        config
            .validate("taiko_hoodi/hoodi")
            .expect("zero rebid price step bps should be accepted");
    }

    #[test]
    fn boundless_pair_config_rejects_excessive_rebid_max_attempts() {
        let config = BoundlessPairConfig {
            rebid_max_attempts: Some(REBID_MAX_ATTEMPTS_LIMIT + 1),
            ..Default::default()
        };

        assert!(
            config
                .validate("taiko_hoodi/hoodi")
                .expect_err("excessive rebid attempts should fail")
                .to_string()
                .contains("rebid_max_attempts")
        );
    }

    #[test]
    fn rpc_pair_beacon_rpc_overrides_l1_chain_spec() {
        let beacon_rpc = "https://beacon.example.test/".to_string();
        let config = RpcConfig {
            pairs: vec![NetworkPairConfig {
                network: "taiko_dev".to_string(),
                l1_network: "taiko_dev_l1".to_string(),
                l1_rpc: Some("https://l1.example.test/".to_string()),
                beacon_rpc: Some(beacon_rpc.clone()),
                l2_rpc: Some("https://l2.example.test/".to_string()),
                l2_provider: L2ProviderKind::Reth,
                l2_witness_rpc: None,
                sp1_verifier_rpc_url: None,
                sp1_verifier_address: None,
                boundless: BoundlessPairConfig::default(),
            }],
            ..Default::default()
        };

        let pair = config
            .resolve_pair("taiko_dev", "taiko_dev_l1")
            .expect("pair should resolve");

        assert_eq!(
            pair.l1_spec.beacon_rpc.as_deref(),
            Some(beacon_rpc.as_str())
        );
    }

    #[test]
    fn rpc_config_rejects_invalid_beacon_rpc_url() {
        let config = RpcConfig {
            pairs: vec![NetworkPairConfig {
                network: "taiko_dev".to_string(),
                l1_network: "taiko_dev_l1".to_string(),
                l1_rpc: Some("https://l1.example.test/".to_string()),
                beacon_rpc: Some("not-a-valid-url".to_string()),
                l2_rpc: Some("https://l2.example.test/".to_string()),
                l2_provider: L2ProviderKind::Reth,
                l2_witness_rpc: None,
                sp1_verifier_rpc_url: None,
                sp1_verifier_address: None,
                boundless: BoundlessPairConfig::default(),
            }],
            ..Default::default()
        };

        let err = config
            .validate()
            .expect_err("invalid beacon rpc should fail");

        assert!(err.to_string().contains("beacon_rpc"));
    }

    #[test]
    fn sp1_verifier_url_error_omits_credentials_query_and_fragment() {
        let sensitive_url = "ftp://sample_user:sample_password@verifier.example.com?api_key=query_secret#fragment_secret";
        let mut config = RpcConfig::default();
        config.pairs[0].sp1_verifier_rpc_url = Some(sensitive_url.to_string());
        config.pairs[0].sp1_verifier_address =
            Some("0x0000000000000000000000000000000000000001".to_string());

        let err = config
            .validate()
            .expect_err("unsupported SP1 verifier URL scheme must fail");
        let message = err.to_string();

        assert!(message.contains("sp1_verifier_rpc_url"));
        assert!(!message.contains(sensitive_url));
        assert!(!message.contains("sample_password"));
        assert!(!message.contains("query_secret"));
        assert!(!message.contains("fragment_secret"));
    }
}
