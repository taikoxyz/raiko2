use std::str::FromStr;

use alloy_primitives::{
    U256,
    utils::{parse_ether, parse_units},
};
use raiko2_primitives::{RaikoError, RaikoResult};
use serde::{Deserialize, Serialize};

const STAKE_TOKEN_DECIMALS: u8 = 18;
const DEFAULT_RISC0_EXECUTION_PO2: u32 = 20;
pub const DEFAULT_REBID_TIMEOUT_MS: u64 = 300_000;
/// Minimum accepted rebid timeout. Config validation rejects anything lower, and the runtime clamps
/// the effective no-lock delay to this floor, so this is the single source of truth for both.
pub const MIN_REBID_TIMEOUT_MS: u64 = 1_000;
pub const DEFAULT_REBID_PRICE_STEP_BPS: u32 = 5000;
pub const DEFAULT_REBID_MAX_ATTEMPTS: u32 = 4;
pub const REBID_MAX_ATTEMPTS_LIMIT: u32 = 31;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DeploymentType {
    Sepolia,
    Base,
    Taiko,
}

impl FromStr for DeploymentType {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_lowercase().as_str() {
            "sepolia" => Ok(Self::Sepolia),
            "base" => Ok(Self::Base),
            "taiko" => Ok(Self::Taiko),
            _ => Err(format!("Invalid boundless deployment_type: {value}")),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BoundlessPricingMode {
    #[default]
    Manual,
    Market,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct BoundlessOfferParams {
    #[serde(default)]
    pub pricing_mode: BoundlessPricingMode,
    pub ramp_up_start_sec: u32,
    pub ramp_up_period_blocks: u32,
    pub lock_timeout_ms_per_mcycle: u32,
    pub timeout_ms_per_mcycle: u32,
    /// Fixed lock timeout in seconds. When set, it replaces the per-mcycle derived
    /// lock timeout and is not scaled by `dynamic_pricing_timeout_modifier`.
    #[serde(default)]
    pub lock_timeout_secs: Option<u32>,
    /// Fixed request timeout in seconds. When set, it replaces the per-mcycle derived
    /// timeout and is not scaled by `dynamic_pricing_timeout_modifier`.
    #[serde(default)]
    pub timeout_secs: Option<u32>,
    #[serde(default)]
    pub dynamic_pricing_timeout_modifier: Option<f64>,
    #[serde(default)]
    pub max_price_per_mcycle: Option<String>,
    #[serde(default)]
    pub min_price_per_mcycle: Option<String>,
    pub lock_collateral: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OfferParamsConfig {
    pub batch: BoundlessOfferParams,
    pub aggregation: BoundlessOfferParams,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeploymentConfig {
    pub deployment_type: Option<DeploymentType>,
    pub overrides: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BatchQuoteStrategy {
    #[default]
    RaikoAgent,
    Evaluated,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BoundlessConfig {
    #[serde(default = "default_execution_po2")]
    pub execution_po2: u32,
    #[serde(default)]
    pub offchain: bool,
    pub rpc_url: String,
    pub signer_key: String,
    #[serde(default)]
    pub deployment: Option<DeploymentConfig>,
    #[serde(default)]
    pub batch_quoted_mcycles: Option<u32>,
    #[serde(default)]
    pub batch_quote_strategy: BatchQuoteStrategy,
    #[serde(default)]
    pub aggregation_quoted_mcycles: Option<u32>,
    #[serde(default)]
    pub aggregation_quote_strategy: BatchQuoteStrategy,
    pub offer_params: OfferParamsConfig,
    #[serde(default = "default_poll_interval_ms")]
    pub poll_interval_ms: u64,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default = "default_rebid_timeout_ms")]
    pub rebid_timeout_ms: u64,
    #[serde(default = "default_rebid_price_step_bps")]
    pub rebid_price_step_bps: u32,
    #[serde(default = "default_rebid_max_attempts")]
    pub rebid_max_attempts: u32,
}

impl Default for BoundlessConfig {
    fn default() -> Self {
        Self {
            execution_po2: default_execution_po2(),
            offchain: false,
            rpc_url: "https://base-rpc.publicnode.com".to_string(),
            signer_key: String::new(),
            deployment: Some(DeploymentConfig {
                deployment_type: Some(DeploymentType::Base),
                overrides: Some(serde_json::json!({
                    "order_stream_url": "https://base-mainnet.boundless.network"
                })),
            }),
            batch_quoted_mcycles: None,
            batch_quote_strategy: BatchQuoteStrategy::default(),
            aggregation_quoted_mcycles: None,
            aggregation_quote_strategy: BatchQuoteStrategy::default(),
            offer_params: OfferParamsConfig {
                batch: default_batch_offer_params(),
                aggregation: default_aggregation_offer_params(),
            },
            poll_interval_ms: default_poll_interval_ms(),
            timeout_ms: default_timeout_ms(),
            rebid_timeout_ms: default_rebid_timeout_ms(),
            rebid_price_step_bps: default_rebid_price_step_bps(),
            rebid_max_attempts: default_rebid_max_attempts(),
        }
    }
}

const fn default_execution_po2() -> u32 {
    DEFAULT_RISC0_EXECUTION_PO2
}

const fn default_poll_interval_ms() -> u64 {
    10_000
}

const fn default_timeout_ms() -> u64 {
    3_600_000
}

const fn default_rebid_timeout_ms() -> u64 {
    DEFAULT_REBID_TIMEOUT_MS
}

const fn default_rebid_price_step_bps() -> u32 {
    DEFAULT_REBID_PRICE_STEP_BPS
}

const fn default_rebid_max_attempts() -> u32 {
    DEFAULT_REBID_MAX_ATTEMPTS
}

// Default offer prices are calibrated against observed Taiko-market clearing data
// (May-Jun 2026): locked requests cleared at a median of ~240 gwei/mcycle (p90 ~890),
// and requests capped below ~100 gwei/mcycle were mostly never locked. Provers also
// post a fixed ~8.7 microETH cost per proof, so requests quoted at few mcycles
// (aggregation, flat 200 mcycles) need a substantially higher per-mcycle cap than
// proposal batches for the same effective margin. The max price is a cap, not the
// paid price: provers lock during the ramp at their own floor whenever there is
// competition.
pub(crate) fn default_batch_offer_params() -> BoundlessOfferParams {
    BoundlessOfferParams {
        pricing_mode: BoundlessPricingMode::Manual,
        ramp_up_start_sec: 20,
        ramp_up_period_blocks: 60,
        lock_timeout_ms_per_mcycle: 300,
        timeout_ms_per_mcycle: 900,
        lock_timeout_secs: None,
        timeout_secs: None,
        dynamic_pricing_timeout_modifier: None,
        max_price_per_mcycle: Some("0.0000006".to_string()),
        min_price_per_mcycle: Some("0.000000010".to_string()),
        lock_collateral: "20".to_string(),
    }
}

fn default_aggregation_offer_params() -> BoundlessOfferParams {
    BoundlessOfferParams {
        pricing_mode: BoundlessPricingMode::Manual,
        ramp_up_start_sec: 20,
        ramp_up_period_blocks: 60,
        lock_timeout_ms_per_mcycle: 3000,
        timeout_ms_per_mcycle: 6000,
        lock_timeout_secs: None,
        timeout_secs: None,
        dynamic_pricing_timeout_modifier: None,
        max_price_per_mcycle: Some("0.0000008".to_string()),
        min_price_per_mcycle: Some("0.000000006".to_string()),
        lock_collateral: "20".to_string(),
    }
}

impl BoundlessConfig {
    #[must_use]
    pub fn get_deployment_type(&self) -> DeploymentType {
        self.deployment
            .as_ref()
            .and_then(|deployment| deployment.deployment_type.clone())
            .unwrap_or(DeploymentType::Base)
    }

    #[must_use]
    pub fn block_time_sec(&self) -> u32 {
        match self.get_deployment_type() {
            DeploymentType::Base => 2,
            DeploymentType::Sepolia => 12,
            DeploymentType::Taiko => 1,
        }
    }
}

fn parse_staking_token(value: &str) -> RaikoResult<U256> {
    parse_units(value, STAKE_TOKEN_DECIMALS)
        .map(Into::into)
        .map_err(|e| {
            RaikoError::InvalidRequestConfig(format!(
                "Failed to parse lock_collateral {value}: {e}"
            ))
        })
}

/// Validate the static offer invariants that must hold for every Boundless offer config.
///
/// # Errors
///
/// Returns an error when the configured min/max price range, timeout ordering, or staking token
/// amount is invalid.
pub fn validate_offer_spec(offer_spec: &BoundlessOfferParams) -> Result<(), String> {
    validate_offer_prices(offer_spec)?;
    validate_dynamic_pricing_timeout_modifier(offer_spec)?;
    if offer_spec.timeout_ms_per_mcycle <= offer_spec.lock_timeout_ms_per_mcycle {
        return Err("timeout must be greater than lock_timeout".to_string());
    }
    if let (Some(lock_timeout_secs), Some(timeout_secs)) =
        (offer_spec.lock_timeout_secs, offer_spec.timeout_secs)
        && timeout_secs <= lock_timeout_secs
    {
        return Err("timeout_secs must be greater than lock_timeout_secs".to_string());
    }
    parse_staking_token(&offer_spec.lock_collateral).map_err(|err| err.to_string())?;
    Ok(())
}

fn validate_offer_prices(offer_spec: &BoundlessOfferParams) -> Result<(), String> {
    match offer_spec.pricing_mode {
        BoundlessPricingMode::Manual => validate_manual_offer_prices(offer_spec),
        BoundlessPricingMode::Market => validate_market_offer_prices(offer_spec),
    }
}

fn validate_manual_offer_prices(offer_spec: &BoundlessOfferParams) -> Result<(), String> {
    let max_price_value = offer_spec
        .max_price_per_mcycle
        .as_deref()
        .ok_or_else(|| "max_price_per_mcycle is required when pricing_mode=manual".to_string())?;
    let max_price = parse_ether(max_price_value)
        .map_err(|e| format!("Failed to parse max_price_per_mcycle {max_price_value}: {e}"))?;
    let min_price_value = offer_spec.min_price_per_mcycle.as_deref().unwrap_or("0");
    let min_price = parse_ether(min_price_value)
        .map_err(|e| format!("Failed to parse min_price_per_mcycle {min_price_value}: {e}"))?;
    if min_price > max_price {
        return Err("min_price_per_mcycle cannot exceed max_price_per_mcycle".to_string());
    }
    Ok(())
}

fn validate_dynamic_pricing_timeout_modifier(
    offer_spec: &BoundlessOfferParams,
) -> Result<(), String> {
    let Some(modifier) = offer_spec.dynamic_pricing_timeout_modifier else {
        return Ok(());
    };
    if offer_spec.pricing_mode != BoundlessPricingMode::Market {
        return Err(
            "dynamic_pricing_timeout_modifier is only valid when pricing_mode=market".to_string(),
        );
    }
    if !modifier.is_finite() || modifier < 1.0 {
        return Err(
            "dynamic_pricing_timeout_modifier must be a finite number greater than or equal to 1.0"
                .to_string(),
        );
    }
    Ok(())
}

fn validate_market_offer_prices(offer_spec: &BoundlessOfferParams) -> Result<(), String> {
    if offer_spec.min_price_per_mcycle.is_some() {
        return Err("min_price_per_mcycle must be omitted when pricing_mode=market".to_string());
    }
    if let Some(max_price_value) = offer_spec.max_price_per_mcycle.as_deref() {
        let max_price_cap = parse_ether(max_price_value)
            .map_err(|e| format!("Failed to parse max_price_per_mcycle {max_price_value}: {e}"))?;
        // Market offers are clamped to this cap, so a zero cap would silently bid 0 wei forever.
        if max_price_cap.is_zero() {
            return Err(
                "max_price_per_mcycle must be positive when set with pricing_mode=market"
                    .to_string(),
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        BoundlessConfig, BoundlessOfferParams, BoundlessPricingMode, DEFAULT_REBID_MAX_ATTEMPTS,
        DEFAULT_REBID_PRICE_STEP_BPS, DEFAULT_REBID_TIMEOUT_MS, DeploymentConfig, DeploymentType,
        validate_offer_spec,
    };

    #[test]
    fn default_config_uses_base_deployment() {
        let config = BoundlessConfig::default();
        assert_eq!(config.get_deployment_type(), DeploymentType::Base);
        assert!(!config.offchain);
        assert_eq!(config.rebid_timeout_ms, DEFAULT_REBID_TIMEOUT_MS);
        assert_eq!(config.rebid_price_step_bps, DEFAULT_REBID_PRICE_STEP_BPS);
        assert_eq!(config.rebid_max_attempts, DEFAULT_REBID_MAX_ATTEMPTS);
    }

    #[test]
    fn taiko_deployment_type_parses_and_uses_taiko_block_time() {
        let deployment_type = "taiko".parse::<DeploymentType>().expect("parse taiko");
        assert_eq!(deployment_type, DeploymentType::Taiko);

        let mut config = BoundlessConfig::default();
        config
            .deployment
            .as_mut()
            .expect("default deployment")
            .deployment_type = Some(deployment_type);

        assert_eq!(config.get_deployment_type(), DeploymentType::Taiko);
        assert_eq!(config.block_time_sec(), 1);
    }

    #[test]
    fn taiko_deployment_type_deserializes_from_config_value() {
        let deployment: DeploymentConfig = serde_json::from_value(serde_json::json!({
            "deployment_type": "taiko"
        }))
        .expect("deserialize taiko deployment");

        assert_eq!(deployment.deployment_type, Some(DeploymentType::Taiko));
    }

    #[test]
    fn default_batch_offer_matches_documented_defaults() {
        let batch = BoundlessConfig::default().offer_params.batch;
        assert_eq!(batch.ramp_up_start_sec, 20);
        assert_eq!(batch.ramp_up_period_blocks, 60);
        assert_eq!(batch.lock_timeout_ms_per_mcycle, 300);
        assert_eq!(batch.timeout_ms_per_mcycle, 900);
        assert_eq!(batch.pricing_mode, BoundlessPricingMode::Manual);
        assert_eq!(batch.max_price_per_mcycle.as_deref(), Some("0.0000006"));
        assert_eq!(batch.min_price_per_mcycle.as_deref(), Some("0.000000010"));
        assert_eq!(batch.lock_collateral, "20");
    }

    #[test]
    fn default_aggregation_offer_matches_documented_defaults() {
        let aggregation = BoundlessConfig::default().offer_params.aggregation;
        assert_eq!(aggregation.ramp_up_start_sec, 20);
        assert_eq!(aggregation.ramp_up_period_blocks, 60);
        assert_eq!(aggregation.lock_timeout_ms_per_mcycle, 3000);
        assert_eq!(aggregation.timeout_ms_per_mcycle, 6000);
        assert_eq!(aggregation.pricing_mode, BoundlessPricingMode::Manual);
        assert_eq!(
            aggregation.max_price_per_mcycle.as_deref(),
            Some("0.0000008")
        );
        assert_eq!(
            aggregation.min_price_per_mcycle.as_deref(),
            Some("0.000000006")
        );
        assert_eq!(aggregation.lock_collateral, "20");
    }

    #[test]
    fn validate_offer_spec_rejects_min_price_above_max_price() {
        let mut offer = BoundlessConfig::default().offer_params.batch;
        offer.min_price_per_mcycle = Some("0.000001".to_string());
        let err = validate_offer_spec(&offer).expect_err("min_price above max");
        assert!(err.contains("min_price_per_mcycle cannot exceed max_price_per_mcycle"));
    }

    #[test]
    fn validate_offer_spec_rejects_timeout_not_above_lock_timeout() {
        let mut offer = BoundlessConfig::default().offer_params.batch;
        offer.timeout_ms_per_mcycle = offer.lock_timeout_ms_per_mcycle;

        let err = validate_offer_spec(&offer).expect_err("timeout not above lock_timeout");
        assert!(err.contains("timeout must be greater than lock_timeout"));
    }

    #[test]
    fn validate_offer_spec_rejects_timeout_secs_not_above_lock_timeout_secs() {
        let mut offer = BoundlessConfig::default().offer_params.batch;
        offer.lock_timeout_secs = Some(600);
        offer.timeout_secs = Some(600);
        let err = validate_offer_spec(&offer).expect_err("timeout_secs not above lock");
        assert!(err.contains("timeout_secs"));
    }

    #[test]
    fn validate_offer_spec_accepts_fixed_timeout_overrides() {
        let mut offer = BoundlessConfig::default().offer_params.batch;
        offer.lock_timeout_secs = Some(600);
        offer.timeout_secs = Some(3600);
        validate_offer_spec(&offer).expect("valid fixed timeout overrides");
    }

    #[test]
    fn offer_params_default_to_no_fixed_timeout_overrides() {
        let batch = BoundlessConfig::default().offer_params.batch;
        assert_eq!(batch.lock_timeout_secs, None);
        assert_eq!(batch.timeout_secs, None);
    }

    #[test]
    fn offer_params_deserialize_fixed_timeout_overrides() {
        let offer: BoundlessOfferParams = serde_json::from_value(serde_json::json!({
            "pricing_mode": "manual",
            "max_price_per_mcycle": "0.0000006",
            "min_price_per_mcycle": "0",
            "ramp_up_start_sec": 0,
            "ramp_up_period_blocks": 180,
            "lock_timeout_ms_per_mcycle": 300,
            "timeout_ms_per_mcycle": 900,
            "lock_collateral": "50",
            "lock_timeout_secs": 600,
            "timeout_secs": 3600
        }))
        .expect("deserialize offer params with fixed timeout overrides");
        assert_eq!(offer.lock_timeout_secs, Some(600));
        assert_eq!(offer.timeout_secs, Some(3600));
    }

    #[test]
    fn validate_offer_spec_accepts_market_pricing_without_manual_prices() {
        let mut offer = BoundlessConfig::default().offer_params.batch;
        offer.pricing_mode = BoundlessPricingMode::Market;
        offer.min_price_per_mcycle = None;
        offer.max_price_per_mcycle = None;

        validate_offer_spec(&offer).expect("valid market offer");
    }

    #[test]
    fn validate_offer_spec_accepts_market_pricing_with_max_cap() {
        let mut offer = BoundlessConfig::default().offer_params.batch;
        offer.pricing_mode = BoundlessPricingMode::Market;
        offer.max_price_per_mcycle = Some("0.000000060".to_string());
        offer.min_price_per_mcycle = None;
        validate_offer_spec(&offer).expect("valid market offer with max cap");
    }

    #[test]
    fn validate_offer_spec_rejects_zero_market_price_cap() {
        let mut offer = BoundlessConfig::default().offer_params.batch;
        offer.pricing_mode = BoundlessPricingMode::Market;
        offer.min_price_per_mcycle = None;
        offer.max_price_per_mcycle = Some("0".to_string());

        let err = validate_offer_spec(&offer).expect_err("zero market cap");
        assert!(err.contains("must be positive"));
    }

    #[test]
    fn validate_offer_spec_rejects_market_pricing_with_min_price() {
        let mut offer = BoundlessConfig::default().offer_params.batch;
        offer.pricing_mode = BoundlessPricingMode::Market;
        offer.max_price_per_mcycle = None;
        let err = validate_offer_spec(&offer).expect_err("market offer with min price");
        assert!(err.contains("min_price_per_mcycle must be omitted"));
    }

    #[test]
    fn validate_offer_spec_accepts_market_pricing_with_timeout_modifier() {
        let mut offer = BoundlessConfig::default().offer_params.batch;
        offer.pricing_mode = BoundlessPricingMode::Market;
        offer.max_price_per_mcycle = None;
        offer.min_price_per_mcycle = None;
        offer.dynamic_pricing_timeout_modifier = Some(2.0);

        validate_offer_spec(&offer).expect("valid market offer with timeout modifier");
    }

    #[test]
    fn validate_offer_spec_rejects_manual_pricing_with_timeout_modifier() {
        let mut offer = BoundlessConfig::default().offer_params.batch;
        offer.dynamic_pricing_timeout_modifier = Some(2.0);

        let err = validate_offer_spec(&offer).expect_err("manual offer with timeout modifier");
        assert!(err.contains("dynamic_pricing_timeout_modifier is only valid"));
    }

    #[test]
    fn validate_offer_spec_rejects_timeout_modifier_below_one() {
        let mut offer = BoundlessConfig::default().offer_params.batch;
        offer.pricing_mode = BoundlessPricingMode::Market;
        offer.max_price_per_mcycle = None;
        offer.min_price_per_mcycle = None;
        offer.dynamic_pricing_timeout_modifier = Some(0.5);

        let err = validate_offer_spec(&offer).expect_err("timeout modifier below one");
        assert!(err.contains("greater than or equal to 1.0"));
    }

    #[test]
    fn validate_offer_spec_rejects_manual_pricing_without_max_price() {
        let mut offer = BoundlessConfig::default().offer_params.batch;
        offer.pricing_mode = BoundlessPricingMode::Manual;
        offer.max_price_per_mcycle = None;

        let err = validate_offer_spec(&offer).expect_err("manual offer without max price");
        assert!(err.contains("max_price_per_mcycle is required"));
    }
}
