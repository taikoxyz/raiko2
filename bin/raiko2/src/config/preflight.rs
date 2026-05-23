use super::ResolvedNetworkPair;
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// Optional preflight behavior configured at server startup.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreflightConfig {
    /// Optional network-keyed cross-check RPCs used to verify preflight checkpoints.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub verify_checkpoint_l2_rpcs: BTreeMap<String, String>,
}

impl PreflightConfig {
    /// # Errors
    ///
    /// Returns an error when the configured verification RPC URL is invalid.
    pub fn validate(&self, resolved_pairs: &[ResolvedNetworkPair]) -> Result<()> {
        let mut pair_count_by_network = HashMap::<&str, usize>::new();
        for pair in resolved_pairs {
            *pair_count_by_network
                .entry(pair.network.as_str())
                .or_default() += 1;
        }

        for (network, url) in &self.verify_checkpoint_l2_rpcs {
            let network = network.trim();
            let url = url.trim();
            if network.is_empty() {
                bail!("preflight.verify_checkpoint_l2_rpcs keys must not be empty");
            }
            if url.is_empty() {
                bail!("preflight.verify_checkpoint_l2_rpcs.{network} must not be empty");
            }
            if !super::rpc::is_valid_url(url) {
                bail!("preflight.verify_checkpoint_l2_rpcs.{network} is not a valid URL: {url}");
            }

            match pair_count_by_network.get(network).copied() {
                Some(1) => {}
                Some(count) if count > 1 => {
                    bail!(
                        "preflight.verify_checkpoint_l2_rpcs.{network} is ambiguous because rpc.pairs contains {count} entries with network='{network}'"
                    );
                }
                _ => {
                    bail!(
                        "preflight.verify_checkpoint_l2_rpcs.{network} does not match any configured rpc.pairs network"
                    );
                }
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn verify_checkpoint_l2_rpc_for_network(&self, network: &str) -> Option<&str> {
        self.verify_checkpoint_l2_rpcs
            .get(network)
            .map(String::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }
}
