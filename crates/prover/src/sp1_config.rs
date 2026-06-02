use alloy_primitives::Address;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use url::Url;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sp1RequestContext {
    ProposalBatch { aggregate: bool },
    Aggregation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sp1ConfigError {
    NetworkOverridesRequireNetworkProver,
    BatchExecuteAggregationNotSupported,
    AggregationExecuteNotSupported,
    CycleLimitMustBePositive,
    ProposalCycleLimitMustBePositive,
    AggregationCycleLimitMustBePositive,
    TimeoutSecsMustBePositive,
    MaxPricePerPguMustBePositive,
    AuctionTimeoutSecsMustBePositive,
    RpcUrlMustNotBeEmpty,
    RpcUrlInvalid(String),
    RemoteVerifyRpcUrlMustNotBeEmpty,
    RemoteVerifyRpcUrlInvalid(String),
    RemoteVerifyAddressMustNotBeEmpty,
    RemoteVerifyAddressInvalid(String),
    ExecuteModeDoesNotSupportNetworkProver,
    MainnetRequiresAuction(Sp1FulfillmentStrategy),
    ReservedRequiresReservedOrHosted(Sp1FulfillmentStrategy),
}

impl std::fmt::Display for Sp1ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NetworkOverridesRequireNetworkProver => {
                f.write_str("sp1 network-only settings require sp1.prover=network")
            }
            Self::BatchExecuteAggregationNotSupported => {
                f.write_str("sp1.mode=execute does not support aggregate=true")
            }
            Self::AggregationExecuteNotSupported => {
                f.write_str("sp1.mode=execute is not supported for aggregation")
            }
            Self::CycleLimitMustBePositive => f.write_str("sp1.cycle_limit must be greater than 0"),
            Self::ProposalCycleLimitMustBePositive => {
                f.write_str("sp1.proposal_cycle_limit must be greater than 0")
            }
            Self::AggregationCycleLimitMustBePositive => {
                f.write_str("sp1.aggregation_cycle_limit must be greater than 0")
            }
            Self::TimeoutSecsMustBePositive => {
                f.write_str("sp1.timeout_secs must be greater than 0")
            }
            Self::MaxPricePerPguMustBePositive => {
                f.write_str("sp1.max_price_per_pgu must be greater than 0")
            }
            Self::AuctionTimeoutSecsMustBePositive => {
                f.write_str("sp1.auction_timeout_secs must be greater than 0")
            }
            Self::RpcUrlMustNotBeEmpty => f.write_str("sp1.rpc_url must not be empty"),
            Self::RpcUrlInvalid(url) => write!(f, "sp1.rpc_url is invalid: {url}"),
            Self::RemoteVerifyRpcUrlMustNotBeEmpty => {
                f.write_str("sp1.remote_verify.rpc_url must not be empty")
            }
            Self::RemoteVerifyRpcUrlInvalid(url) => {
                write!(f, "sp1.remote_verify.rpc_url is invalid: {url}")
            }
            Self::RemoteVerifyAddressMustNotBeEmpty => {
                f.write_str("sp1.remote_verify.verifier_address must not be empty")
            }
            Self::RemoteVerifyAddressInvalid(address) => {
                write!(
                    f,
                    "sp1.remote_verify.verifier_address is invalid: {address}"
                )
            }
            Self::ExecuteModeDoesNotSupportNetworkProver => {
                f.write_str("sp1.mode=execute does not support sp1.prover=network")
            }
            Self::MainnetRequiresAuction(strategy) => write!(
                f,
                "sp1.network_mode=mainnet requires sp1.fulfillment_strategy=auction, got {strategy}"
            ),
            Self::ReservedRequiresReservedOrHosted(strategy) => write!(
                f,
                "sp1.network_mode=reserved requires sp1.fulfillment_strategy=reserved or hosted, got {strategy}"
            ),
        }
    }
}

impl std::error::Error for Sp1ConfigError {}

/// SP1 prover configuration parameters.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sp1Config {
    /// Proof mode (Core, Compressed, Plonk).
    #[serde(default)]
    pub recursion: RecursionMode,
    /// Prover mode (Mock, Local, Network).
    #[serde(default = "default_prover_mode")]
    pub prover: ProverMode,
    /// Execution mode (prove or execute-only).
    #[serde(default)]
    pub mode: ExecutionMode,
    /// Whether to verify the proof after generation.
    #[serde(default = "default_true")]
    pub verify: bool,
    /// Succinct network mode to use for remote proving.
    #[serde(default)]
    pub network_mode: Sp1NetworkMode,
    /// Succinct fulfillment strategy to use for remote proving.
    #[serde(default)]
    pub fulfillment_strategy: Sp1FulfillmentStrategy,
    /// Skip local simulation before submitting a network prove request.
    #[serde(default = "default_true")]
    pub skip_simulation: bool,
    /// Cycle limit to attach to network prove requests.
    #[serde(default = "default_cycle_limit")]
    pub cycle_limit: u64,
    /// Optional proposal-stage cycle limit. Falls back to `cycle_limit`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposal_cycle_limit: Option<u64>,
    /// Optional aggregation-stage cycle limit. Falls back to `cycle_limit`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aggregation_cycle_limit: Option<u64>,
    /// Timeout to use when waiting for network proofs.
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    /// Optional max price per PGU to attach to SP1 network proof requests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_price_per_pgu: Option<u64>,
    /// Optional assignment wait timeout before retrying an SP1 network proof request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auction_timeout_secs: Option<u64>,
    /// Optional override for the Succinct network RPC URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rpc_url: Option<String>,
    /// Optional remote verifier contract configuration for hosted SP1 verification.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_verify: Option<Sp1RemoteVerifyConfig>,
}

const fn default_true() -> bool {
    true
}

const fn default_prover_mode() -> ProverMode {
    ProverMode::Network
}

const fn default_cycle_limit() -> u64 {
    1_000_000_000_000
}

const fn default_timeout_secs() -> u64 {
    7_200
}

impl Default for Sp1Config {
    fn default() -> Self {
        Self {
            recursion: RecursionMode::Plonk,
            prover: default_prover_mode(),
            mode: ExecutionMode::Prove,
            verify: true,
            network_mode: Sp1NetworkMode::Reserved,
            fulfillment_strategy: Sp1FulfillmentStrategy::Reserved,
            skip_simulation: true,
            cycle_limit: default_cycle_limit(),
            proposal_cycle_limit: None,
            aggregation_cycle_limit: None,
            timeout_secs: default_timeout_secs(),
            max_price_per_pgu: None,
            auction_timeout_secs: None,
            rpc_url: None,
            remote_verify: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(deny_unknown_fields)]
pub struct Sp1RemoteVerifyConfig {
    pub rpc_url: String,
    pub verifier_address: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(deny_unknown_fields)]
pub struct Sp1SystemConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_verify: Option<Sp1RemoteVerifyConfig>,
}

/// Request-scoped SP1 overrides accepted via `prover_args.sp1`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(deny_unknown_fields)]
pub struct Sp1ConfigOverrides {
    #[serde(default)]
    pub recursion: Option<RecursionMode>,
    #[serde(default)]
    pub prover: Option<ProverMode>,
    #[serde(default)]
    pub mode: Option<ExecutionMode>,
    #[serde(default)]
    pub verify: Option<bool>,
    #[serde(default)]
    pub network_mode: Option<Sp1NetworkMode>,
    #[serde(default)]
    pub fulfillment_strategy: Option<Sp1FulfillmentStrategy>,
    #[serde(default)]
    pub skip_simulation: Option<bool>,
    #[serde(default)]
    pub cycle_limit: Option<u64>,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    #[serde(default)]
    pub max_price_per_pgu: Option<u64>,
    #[serde(default)]
    pub auction_timeout_secs: Option<u64>,
}

impl Sp1Config {
    #[must_use]
    pub fn merged_with(&self, overrides: &Sp1ConfigOverrides) -> Self {
        Self {
            recursion: overrides.recursion.unwrap_or(self.recursion),
            prover: overrides.prover.unwrap_or(self.prover),
            mode: overrides.mode.unwrap_or(self.mode),
            verify: overrides.verify.unwrap_or(self.verify),
            network_mode: overrides.network_mode.unwrap_or(self.network_mode),
            fulfillment_strategy: overrides
                .fulfillment_strategy
                .unwrap_or(self.fulfillment_strategy),
            skip_simulation: overrides.skip_simulation.unwrap_or(self.skip_simulation),
            cycle_limit: overrides.cycle_limit.unwrap_or(self.cycle_limit),
            proposal_cycle_limit: self.proposal_cycle_limit,
            aggregation_cycle_limit: self.aggregation_cycle_limit,
            timeout_secs: overrides.timeout_secs.unwrap_or(self.timeout_secs),
            max_price_per_pgu: overrides.max_price_per_pgu.or(self.max_price_per_pgu),
            auction_timeout_secs: overrides.auction_timeout_secs.or(self.auction_timeout_secs),
            rpc_url: self.rpc_url.clone(),
            remote_verify: self.remote_verify.clone(),
        }
    }

    /// # Errors
    ///
    /// Returns an error when request-scoped overrides produce an invalid SP1 configuration.
    pub fn resolve_request_config(
        &self,
        overrides: Option<&Sp1ConfigOverrides>,
        context: Sp1RequestContext,
    ) -> Result<Self, Sp1ConfigError> {
        let empty_overrides = Sp1ConfigOverrides::default();
        let overrides = overrides.unwrap_or(&empty_overrides);
        let mut effective_config = self.merged_with(overrides);
        if overrides.cycle_limit.is_none() {
            effective_config.cycle_limit = effective_config.cycle_limit_for_context(context);
        }
        match context {
            Sp1RequestContext::ProposalBatch { .. }
                if effective_config.mode == ExecutionMode::Prove =>
            {
                effective_config.recursion = RecursionMode::Compressed;
            }
            Sp1RequestContext::Aggregation => {
                effective_config.recursion = RecursionMode::Plonk;
            }
            Sp1RequestContext::ProposalBatch { .. } => {}
        }
        if overrides.has_network_overrides() && effective_config.prover != ProverMode::Network {
            return Err(Sp1ConfigError::NetworkOverridesRequireNetworkProver);
        }
        if effective_config.mode == ExecutionMode::Execute {
            match context {
                Sp1RequestContext::ProposalBatch { aggregate: true } => {
                    return Err(Sp1ConfigError::BatchExecuteAggregationNotSupported);
                }
                Sp1RequestContext::Aggregation => {
                    return Err(Sp1ConfigError::AggregationExecuteNotSupported);
                }
                Sp1RequestContext::ProposalBatch { aggregate: false } => {}
            }
        }
        effective_config.validate()?;
        Ok(effective_config)
    }

    /// # Errors
    ///
    /// Returns an error when the SP1 prover configuration is internally inconsistent.
    pub fn validate(&self) -> Result<(), Sp1ConfigError> {
        if self.cycle_limit == 0 {
            return Err(Sp1ConfigError::CycleLimitMustBePositive);
        }
        if self.proposal_cycle_limit == Some(0) {
            return Err(Sp1ConfigError::ProposalCycleLimitMustBePositive);
        }
        if self.aggregation_cycle_limit == Some(0) {
            return Err(Sp1ConfigError::AggregationCycleLimitMustBePositive);
        }
        if self.timeout_secs == 0 {
            return Err(Sp1ConfigError::TimeoutSecsMustBePositive);
        }
        if self.max_price_per_pgu == Some(0) {
            return Err(Sp1ConfigError::MaxPricePerPguMustBePositive);
        }
        if self.auction_timeout_secs == Some(0) {
            return Err(Sp1ConfigError::AuctionTimeoutSecsMustBePositive);
        }
        if self
            .rpc_url
            .as_deref()
            .is_some_and(|rpc_url| rpc_url.trim().is_empty())
        {
            return Err(Sp1ConfigError::RpcUrlMustNotBeEmpty);
        }
        if let Some(rpc_url) = self.rpc_url.as_deref() {
            if rpc_url != rpc_url.trim() {
                return Err(Sp1ConfigError::RpcUrlInvalid(rpc_url.to_string()));
            }
            Url::parse(rpc_url).map_err(|_| Sp1ConfigError::RpcUrlInvalid(rpc_url.to_string()))?;
        }
        if let Some(remote_verify) = &self.remote_verify {
            if remote_verify.rpc_url.trim().is_empty() {
                return Err(Sp1ConfigError::RemoteVerifyRpcUrlMustNotBeEmpty);
            }
            Url::parse(&remote_verify.rpc_url).map_err(|_| {
                Sp1ConfigError::RemoteVerifyRpcUrlInvalid(remote_verify.rpc_url.clone())
            })?;
            if remote_verify.verifier_address.trim().is_empty() {
                return Err(Sp1ConfigError::RemoteVerifyAddressMustNotBeEmpty);
            }
            Address::from_str(&remote_verify.verifier_address).map_err(|_| {
                Sp1ConfigError::RemoteVerifyAddressInvalid(remote_verify.verifier_address.clone())
            })?;
        }
        if self.mode == ExecutionMode::Execute && self.prover == ProverMode::Network {
            return Err(Sp1ConfigError::ExecuteModeDoesNotSupportNetworkProver);
        }
        if self.prover == ProverMode::Network {
            match (self.network_mode, self.fulfillment_strategy) {
                (Sp1NetworkMode::Mainnet, Sp1FulfillmentStrategy::Auction)
                | (
                    Sp1NetworkMode::Reserved,
                    Sp1FulfillmentStrategy::Reserved | Sp1FulfillmentStrategy::Hosted,
                ) => {}
                (Sp1NetworkMode::Mainnet, strategy) => {
                    return Err(Sp1ConfigError::MainnetRequiresAuction(strategy));
                }
                (Sp1NetworkMode::Reserved, strategy) => {
                    return Err(Sp1ConfigError::ReservedRequiresReservedOrHosted(strategy));
                }
            }
        }

        Ok(())
    }

    const fn cycle_limit_for_context(&self, context: Sp1RequestContext) -> u64 {
        match context {
            Sp1RequestContext::ProposalBatch { .. } => match self.proposal_cycle_limit {
                Some(cycle_limit) => cycle_limit,
                None => self.cycle_limit,
            },
            Sp1RequestContext::Aggregation => match self.aggregation_cycle_limit {
                Some(cycle_limit) => cycle_limit,
                None => self.cycle_limit,
            },
        }
    }
}

impl Sp1ConfigOverrides {
    #[must_use]
    pub const fn has_network_overrides(&self) -> bool {
        self.network_mode.is_some()
            || self.fulfillment_strategy.is_some()
            || self.skip_simulation.is_some()
            || self.timeout_secs.is_some()
            || self.max_price_per_pgu.is_some()
            || self.auction_timeout_secs.is_some()
    }
}

impl Sp1SystemConfig {
    #[must_use]
    pub fn applied_to(&self, config: &Sp1Config) -> Sp1Config {
        let mut config = config.clone();
        if let Some(remote_verify) = &self.remote_verify {
            config.remote_verify = Some(remote_verify.clone());
        }
        config
    }
}

/// SP1 proof recursion mode.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, Default, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum RecursionMode {
    /// Core proof (no recursion).
    Core,
    /// Compressed proof.
    Compressed,
    /// Plonk proof (on-chain verifiable).
    #[default]
    Plonk,
}

/// SP1 proposal execution mode.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, Default, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionMode {
    /// Execute the guest without producing a proof.
    Execute,
    /// Produce a proof through the configured prover.
    #[default]
    Prove,
}

impl ExecutionMode {
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Execute => "execute",
            Self::Prove => "prove",
        }
    }
}

/// SP1 prover mode.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum ProverMode {
    /// Mock prover for testing.
    Mock,
    /// Local CPU prover.
    Local,
    /// Network prover (Succinct network).
    Network,
}

/// SP1 network mode.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, Default, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Sp1NetworkMode {
    /// Reserved capacity network for hosted or reserved proving.
    #[default]
    Reserved,
    /// Mainnet auction-based network.
    Mainnet,
}

impl Sp1NetworkMode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Reserved => "reserved",
            Self::Mainnet => "mainnet",
        }
    }
}

impl std::fmt::Display for Sp1NetworkMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// SP1 fulfillment strategy.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, Default, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Sp1FulfillmentStrategy {
    /// Reserved capacity strategy.
    #[default]
    Reserved,
    /// Hosted proving strategy.
    Hosted,
    /// Auction-based proving strategy.
    Auction,
}

impl Sp1FulfillmentStrategy {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Reserved => "reserved",
            Self::Hosted => "hosted",
            Self::Auction => "auction",
        }
    }
}

impl std::fmt::Display for Sp1FulfillmentStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Sp1NetworkSubmissionProgress {
    pub provider_request_id: String,
    pub network_mode: Sp1NetworkMode,
    pub fulfillment_strategy: Sp1FulfillmentStrategy,
    pub skip_simulation: bool,
    pub cycle_limit: u64,
    pub timeout_secs: u64,
    pub max_price_per_pgu: Option<u64>,
    pub auction_timeout_secs: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Sp1NetworkMetadata {
    pub request_id: String,
    pub network_mode: Sp1NetworkMode,
    pub fulfillment_strategy: Sp1FulfillmentStrategy,
    pub skip_simulation: bool,
    pub cycle_limit: u64,
    pub timeout_secs: u64,
    pub max_price_per_pgu: Option<u64>,
    pub auction_timeout_secs: Option<u64>,
}

impl Sp1NetworkMetadata {
    #[must_use]
    pub const fn from_config(request_id: String, config: &Sp1Config) -> Self {
        Self {
            request_id,
            network_mode: config.network_mode,
            fulfillment_strategy: config.fulfillment_strategy,
            skip_simulation: config.skip_simulation,
            cycle_limit: config.cycle_limit,
            timeout_secs: config.timeout_secs,
            max_price_per_pgu: config.max_price_per_pgu,
            auction_timeout_secs: config.auction_timeout_secs,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ExecutionMode, ProverMode, Sp1Config, Sp1ConfigError, Sp1ConfigOverrides,
        Sp1FulfillmentStrategy, Sp1NetworkMode,
    };

    #[test]
    fn sp1_default_config_is_valid() {
        assert!(Sp1Config::default().validate().is_ok());
    }

    #[test]
    fn sp1_execute_rejects_network_prover() {
        let config = Sp1Config {
            mode: ExecutionMode::Execute,
            prover: ProverMode::Network,
            ..Sp1Config::default()
        };

        let err = config
            .validate()
            .expect_err("execute should reject network");
        assert_eq!(err, Sp1ConfigError::ExecuteModeDoesNotSupportNetworkProver);
    }

    #[test]
    fn sp1_mainnet_requires_auction_strategy() {
        let config = Sp1Config {
            network_mode: Sp1NetworkMode::Mainnet,
            fulfillment_strategy: Sp1FulfillmentStrategy::Reserved,
            ..Sp1Config::default()
        };

        let err = config
            .validate()
            .expect_err("mainnet should reject reserved strategy");
        assert_eq!(
            err,
            Sp1ConfigError::MainnetRequiresAuction(Sp1FulfillmentStrategy::Reserved)
        );
    }

    #[test]
    fn sp1_network_override_detection_ignores_local_fields() {
        let overrides = Sp1ConfigOverrides {
            recursion: Some(super::RecursionMode::Core),
            prover: Some(ProverMode::Local),
            mode: Some(ExecutionMode::Prove),
            verify: Some(true),
            ..Sp1ConfigOverrides::default()
        };

        assert!(!overrides.has_network_overrides());
    }

    #[test]
    fn sp1_network_override_detection_catches_network_fields() {
        let overrides = Sp1ConfigOverrides {
            network_mode: Some(Sp1NetworkMode::Mainnet),
            ..Sp1ConfigOverrides::default()
        };

        assert!(overrides.has_network_overrides());
    }
}
