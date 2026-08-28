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
/// Smallest non-zero rebid step config validation accepts (1%). `0` is a valid, explicit flat
/// ladder, but a value in `1..MIN` is almost always a basis-points/multiplier confusion (e.g. `2`
/// meant as "×2", which is really +0.02%/rung) — a curve so flat it re-creates the same-price-retry
/// pathology bounded rebids were added to avoid, so it is rejected rather than silently honored.
pub const MIN_MEANINGFUL_REBID_PRICE_STEP_BPS: u32 = 100;
pub const DEFAULT_REBID_MAX_ATTEMPTS: u32 = 4;
pub const REBID_MAX_ATTEMPTS_LIMIT: u32 = 31;
pub const BOUNDLESS_TX_MIN_FEE_BUMP_BPS: u32 = 1_000;
pub const BOUNDLESS_TX_MAX_RECEIPT_TIMEOUT_MS: u64 = 90_000;
pub const BOUNDLESS_TX_MAX_TOTAL_ATTEMPT_MS: u64 = 600_000;
pub const BOUNDLESS_TX_SEND_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_BOUNDLESS_TX_RECEIPT_TIMEOUT_MS: u64 = 90_000;
const DEFAULT_BOUNDLESS_TX_FEE_BUMP_BPS: u32 = 5_000;
const DEFAULT_BOUNDLESS_TX_MAX_REPLACEMENTS: u32 = 4;

const fn default_boundless_tx_receipt_timeout_ms() -> u64 {
    DEFAULT_BOUNDLESS_TX_RECEIPT_TIMEOUT_MS
}

const fn default_boundless_tx_fee_bump_bps() -> u32 {
    DEFAULT_BOUNDLESS_TX_FEE_BUMP_BPS
}

const fn default_boundless_tx_max_replacements() -> u32 {
    DEFAULT_BOUNDLESS_TX_MAX_REPLACEMENTS
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BoundlessTransactionConfig {
    #[serde(default = "default_boundless_tx_receipt_timeout_ms")]
    pub receipt_timeout_ms: u64,
    #[serde(default = "default_boundless_tx_fee_bump_bps")]
    pub fee_bump_bps: u32,
    #[serde(default = "default_boundless_tx_max_replacements")]
    pub max_replacements: u32,
    pub max_fee_per_gas_wei: String,
}

impl BoundlessTransactionConfig {
    /// Parse and validate the configured EIP-1559 fee ceiling.
    ///
    /// # Errors
    ///
    /// Returns an error when a transaction timing or fee bound is zero, excessive, or malformed.
    /// `max_replacements = 0` is valid and selects one broadcast attempt without replacements.
    pub fn validate(&self) -> Result<u128, String> {
        if self.receipt_timeout_ms == 0 {
            return Err("receipt_timeout_ms must be greater than zero".to_string());
        }
        if self.receipt_timeout_ms > BOUNDLESS_TX_MAX_RECEIPT_TIMEOUT_MS {
            return Err(format!(
                "receipt_timeout_ms must be <= {BOUNDLESS_TX_MAX_RECEIPT_TIMEOUT_MS}"
            ));
        }
        if self.fee_bump_bps < BOUNDLESS_TX_MIN_FEE_BUMP_BPS {
            return Err(format!(
                "fee_bump_bps must be >= {BOUNDLESS_TX_MIN_FEE_BUMP_BPS}"
            ));
        }
        let total_attempts = u64::from(self.max_replacements).saturating_add(1);
        let total_attempt_ms = self
            .receipt_timeout_ms
            .saturating_add(BOUNDLESS_TX_SEND_TIMEOUT_MS)
            .saturating_mul(total_attempts);
        if total_attempt_ms > BOUNDLESS_TX_MAX_TOTAL_ATTEMPT_MS {
            return Err(format!(
                "(max_replacements + 1) * (receipt_timeout_ms + {BOUNDLESS_TX_SEND_TIMEOUT_MS}) must be <= {BOUNDLESS_TX_MAX_TOTAL_ATTEMPT_MS}"
            ));
        }
        let max_fee_per_gas = self
            .max_fee_per_gas_wei
            .parse::<u128>()
            .map_err(|_| "max_fee_per_gas_wei must be a decimal u128 string".to_string())?;
        if max_fee_per_gas == 0 {
            return Err("max_fee_per_gas_wei must be greater than zero".to_string());
        }
        Ok(max_fee_per_gas)
    }
}

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

/// How the lock/request timeouts for an offer are chosen.
///
/// `PerMcycle` derives both timeouts from the quoted mcycles and, in market mode, may scale
/// them by `dynamic_pricing_timeout_modifier`. `Fixed` pins both timeouts to explicit seconds
/// and is never scaled. Because the modifier lives inside `PerMcycle`, a fixed policy cannot
/// carry a modifier — the two are mutually exclusive by construction.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum TimeoutPolicy {
    PerMcycle {
        lock_timeout_ms_per_mcycle: u32,
        timeout_ms_per_mcycle: u32,
        #[serde(default)]
        dynamic_pricing_timeout_modifier: Option<f64>,
    },
    Fixed {
        lock_timeout_secs: u32,
        timeout_secs: u32,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
// Fail closed on stale/renamed offer-level keys (matching the bin-level config posture). The
// hard cutover deleted several offer fields; without this, a config that leaves e.g.
// `dynamic_pricing_timeout_modifier` at its old offer level — instead of inside `[timeouts]` —
// would boot clean with the value silently ignored. NOTE: this does not reach inside the
// internally-tagged `timeouts`/`*_quote` enums, which serde cannot deny unknown fields on; stale
// keys nested in those tables are still dropped silently (see the migration notes in docs/API.md).
#[serde(deny_unknown_fields)]
pub struct BoundlessOfferParams {
    #[serde(default)]
    pub pricing_mode: BoundlessPricingMode,
    pub ramp_up_start_sec: u32,
    pub ramp_up_period_sec: u32,
    pub timeouts: TimeoutPolicy,
    #[serde(default)]
    pub max_price_per_mcycle: Option<String>,
    #[serde(default)]
    pub min_price_per_mcycle: Option<String>,
    /// Absolute per-mcycle ceiling on the offer's max price: no attempt, initial or
    /// rebid-escalated, ever bids above this value times the quoted mcycles, in either pricing
    /// mode. In `manual` mode it bounds rebid escalation of `max_price_per_mcycle`, which is
    /// otherwise unbounded by config. In `market` mode it is the canonical name for the price
    /// cap and is interchangeable with — but mutually exclusive of — `max_price_per_mcycle`.
    #[serde(default)]
    pub absolute_max_price_per_mcycle: Option<String>,
    pub lock_collateral: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfferParamsConfig {
    pub batch: BoundlessOfferParams,
    pub aggregation: BoundlessOfferParams,
}

// Fail closed on stale/typo'd keys inside `[prover.risc0.boundless.deployment]`. Without this, a typo
// such as `deployment_typ` deserializes with `deployment_type = None`, and `get_deployment_type()`
// silently falls back to `Base` — booting the wrong market for a Taiko/Sepolia deployment. The
// free-form `overrides` value stays unrestricted (it deserializes into a `serde_json::Value`).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeploymentConfig {
    pub deployment_type: Option<DeploymentType>,
    pub overrides: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "strategy", rename_all = "snake_case")]
pub enum QuoteSizing {
    Estimated,
    #[default]
    Evaluated,
    Fixed {
        mcycles: u32,
    },
}

impl QuoteSizing {
    /// Validate the fixed-quote value.
    ///
    /// # Errors
    /// Returns an error when a `Fixed` quote is zero.
    pub fn validate(&self, field: &str) -> Result<(), String> {
        if let QuoteSizing::Fixed { mcycles } = self
            && *mcycles == 0
        {
            return Err(format!("{field} fixed mcycles must be greater than 0"));
        }
        Ok(())
    }
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
    pub transaction: Option<BoundlessTransactionConfig>,
    #[serde(default)]
    pub deployment: Option<DeploymentConfig>,
    pub batch_quote: QuoteSizing,
    pub aggregation_quote: QuoteSizing,
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
            transaction: None,
            deployment: Some(DeploymentConfig {
                deployment_type: Some(DeploymentType::Base),
                overrides: Some(serde_json::json!({
                    "order_stream_url": "https://base-mainnet.boundless.network"
                })),
            }),
            batch_quote: QuoteSizing::default(),
            aggregation_quote: QuoteSizing::default(),
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
        ramp_up_period_sec: 120,
        timeouts: TimeoutPolicy::PerMcycle {
            lock_timeout_ms_per_mcycle: 300,
            timeout_ms_per_mcycle: 900,
            dynamic_pricing_timeout_modifier: None,
        },
        max_price_per_mcycle: Some("0.0000006".to_string()),
        min_price_per_mcycle: Some("0.000000010".to_string()),
        absolute_max_price_per_mcycle: None,
        lock_collateral: "20".to_string(),
    }
}

fn default_aggregation_offer_params() -> BoundlessOfferParams {
    BoundlessOfferParams {
        pricing_mode: BoundlessPricingMode::Manual,
        ramp_up_start_sec: 20,
        ramp_up_period_sec: 120,
        timeouts: TimeoutPolicy::PerMcycle {
            lock_timeout_ms_per_mcycle: 3000,
            timeout_ms_per_mcycle: 6000,
            dynamic_pricing_timeout_modifier: None,
        },
        max_price_per_mcycle: Some("0.0000008".to_string()),
        min_price_per_mcycle: Some("0.000000006".to_string()),
        absolute_max_price_per_mcycle: None,
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
    match &offer_spec.timeouts {
        TimeoutPolicy::PerMcycle {
            lock_timeout_ms_per_mcycle,
            timeout_ms_per_mcycle,
            dynamic_pricing_timeout_modifier,
        } => {
            if timeout_ms_per_mcycle <= lock_timeout_ms_per_mcycle {
                return Err(
                    "timeout_ms_per_mcycle must be greater than lock_timeout_ms_per_mcycle"
                        .to_string(),
                );
            }
            if let Some(modifier) = dynamic_pricing_timeout_modifier {
                if offer_spec.pricing_mode != BoundlessPricingMode::Market {
                    return Err(
                        "dynamic_pricing_timeout_modifier is only valid when pricing_mode=market"
                            .to_string(),
                    );
                }
                if !modifier.is_finite() || *modifier < 1.0 {
                    return Err(
                        "dynamic_pricing_timeout_modifier must be a finite number greater than or equal to 1.0"
                            .to_string(),
                    );
                }
            }
        }
        TimeoutPolicy::Fixed {
            lock_timeout_secs,
            timeout_secs,
        } => {
            if timeout_secs <= lock_timeout_secs {
                return Err("timeout_secs must be greater than lock_timeout_secs".to_string());
            }
        }
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
    if let Some(ceiling_value) = offer_spec.absolute_max_price_per_mcycle.as_deref() {
        let ceiling = parse_ether(ceiling_value).map_err(|e| {
            format!("Failed to parse absolute_max_price_per_mcycle {ceiling_value}: {e}")
        })?;
        // A ceiling below the configured max would make the very first attempt overbid it.
        if ceiling < max_price {
            return Err(
                "absolute_max_price_per_mcycle cannot be below max_price_per_mcycle".to_string(),
            );
        }
    }
    Ok(())
}

fn validate_market_offer_prices(offer_spec: &BoundlessOfferParams) -> Result<(), String> {
    if offer_spec.min_price_per_mcycle.is_some() {
        return Err("min_price_per_mcycle must be omitted when pricing_mode=market".to_string());
    }
    let (cap_field, cap_value) = match (
        offer_spec.absolute_max_price_per_mcycle.as_deref(),
        offer_spec.max_price_per_mcycle.as_deref(),
    ) {
        (Some(_), Some(_)) => {
            return Err("set only one of absolute_max_price_per_mcycle and \
                 max_price_per_mcycle when pricing_mode=market; both name the same cap"
                .to_string());
        }
        (Some(value), None) => ("absolute_max_price_per_mcycle", Some(value)),
        (None, value) => ("max_price_per_mcycle", value),
    };
    if let Some(cap_value) = cap_value {
        let max_price_cap = parse_ether(cap_value)
            .map_err(|e| format!("Failed to parse {cap_field} {cap_value}: {e}"))?;
        // Market offers are clamped to this cap, so a zero cap would silently bid 0 wei forever.
        if max_price_cap.is_zero() {
            return Err(format!(
                "{cap_field} must be positive when set with pricing_mode=market"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        BoundlessConfig, BoundlessOfferParams, BoundlessPricingMode, BoundlessTransactionConfig,
        DEFAULT_REBID_MAX_ATTEMPTS, DEFAULT_REBID_PRICE_STEP_BPS, DEFAULT_REBID_TIMEOUT_MS,
        DeploymentConfig, DeploymentType, QuoteSizing, TimeoutPolicy, validate_offer_spec,
    };

    #[test]
    fn quote_sizing_deserializes_estimated_strategy() {
        let quote: QuoteSizing = serde_json::from_value(serde_json::json!({
            "strategy": "estimated"
        }))
        .expect("estimated quote sizing should deserialize");

        assert_eq!(quote, QuoteSizing::Estimated);
    }

    #[test]
    fn quote_sizing_rejects_removed_raiko_agent_strategy() {
        let error = serde_json::from_value::<QuoteSizing>(serde_json::json!({
            "strategy": "raiko_agent"
        }))
        .expect_err("raiko_agent must no longer be a supported quote strategy");

        assert!(error.to_string().contains("estimated"), "{error}");
        assert!(error.to_string().contains("evaluated"), "{error}");
        assert!(error.to_string().contains("fixed"), "{error}");
    }

    #[test]
    fn explicit_boundless_config_requires_both_quote_stages() {
        for missing_quote in ["batch_quote", "aggregation_quote"] {
            let mut value = serde_json::to_value(BoundlessConfig::default())
                .expect("serialize default Boundless config");
            value
                .as_object_mut()
                .expect("Boundless config object")
                .remove(missing_quote);

            let error = serde_json::from_value::<BoundlessConfig>(value)
                .expect_err("explicit Boundless config must require both quote stages");
            assert!(error.to_string().contains(missing_quote), "{error}");
        }
    }

    #[test]
    fn default_config_uses_base_deployment() {
        let config = BoundlessConfig::default();
        assert_eq!(config.get_deployment_type(), DeploymentType::Base);
        assert!(!config.offchain);
        assert_eq!(config.rebid_timeout_ms, DEFAULT_REBID_TIMEOUT_MS);
        assert_eq!(config.rebid_price_step_bps, DEFAULT_REBID_PRICE_STEP_BPS);
        assert_eq!(config.rebid_max_attempts, DEFAULT_REBID_MAX_ATTEMPTS);
        assert_eq!(config.batch_quote, QuoteSizing::Evaluated);
        assert_eq!(config.aggregation_quote, QuoteSizing::Evaluated);
    }

    #[test]
    fn taiko_deployment_type_parses() {
        let deployment_type = "taiko".parse::<DeploymentType>().expect("parse taiko");
        assert_eq!(deployment_type, DeploymentType::Taiko);
        let mut config = BoundlessConfig::default();
        config
            .deployment
            .as_mut()
            .expect("default deployment")
            .deployment_type = Some(deployment_type);
        assert_eq!(config.get_deployment_type(), DeploymentType::Taiko);
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
    fn deployment_config_rejects_unknown_key() {
        // A typo'd key inside [prover.risc0.boundless.deployment] (here `deployment_typ`) must fail
        // closed rather than deserialize with `deployment_type = None` and silently fall back to
        // the Base deployment.
        let err = serde_json::from_value::<DeploymentConfig>(serde_json::json!({
            "deployment_typ": "taiko"
        }))
        .expect_err("unknown deployment key should be rejected");
        assert!(err.to_string().contains("deployment_typ"));
    }

    #[test]
    fn default_batch_offer_matches_documented_defaults() {
        let batch = BoundlessConfig::default().offer_params.batch;
        assert_eq!(batch.ramp_up_start_sec, 20);
        assert_eq!(batch.ramp_up_period_sec, 120);
        assert_eq!(
            batch.timeouts,
            TimeoutPolicy::PerMcycle {
                lock_timeout_ms_per_mcycle: 300,
                timeout_ms_per_mcycle: 900,
                dynamic_pricing_timeout_modifier: None,
            }
        );
        assert_eq!(batch.pricing_mode, BoundlessPricingMode::Manual);
        assert_eq!(batch.max_price_per_mcycle.as_deref(), Some("0.0000006"));
        assert_eq!(batch.min_price_per_mcycle.as_deref(), Some("0.000000010"));
        assert_eq!(batch.absolute_max_price_per_mcycle, None);
        assert_eq!(batch.lock_collateral, "20");
    }

    #[test]
    fn default_aggregation_offer_matches_documented_defaults() {
        let aggregation = BoundlessConfig::default().offer_params.aggregation;
        assert_eq!(aggregation.ramp_up_start_sec, 20);
        assert_eq!(aggregation.ramp_up_period_sec, 120);
        assert_eq!(
            aggregation.timeouts,
            TimeoutPolicy::PerMcycle {
                lock_timeout_ms_per_mcycle: 3000,
                timeout_ms_per_mcycle: 6000,
                dynamic_pricing_timeout_modifier: None,
            }
        );
        assert_eq!(aggregation.pricing_mode, BoundlessPricingMode::Manual);
        assert_eq!(
            aggregation.max_price_per_mcycle.as_deref(),
            Some("0.0000008")
        );
        assert_eq!(
            aggregation.min_price_per_mcycle.as_deref(),
            Some("0.000000006")
        );
        assert_eq!(aggregation.absolute_max_price_per_mcycle, None);
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
    fn timeout_policy_per_mcycle_requires_timeout_above_lock() {
        let mut offer = BoundlessConfig::default().offer_params.batch;
        offer.timeouts = super::TimeoutPolicy::PerMcycle {
            lock_timeout_ms_per_mcycle: 900,
            timeout_ms_per_mcycle: 900,
            dynamic_pricing_timeout_modifier: None,
        };
        let err = validate_offer_spec(&offer).expect_err("timeout not above lock");
        assert!(
            err.contains("timeout_ms_per_mcycle must be greater than lock_timeout_ms_per_mcycle")
        );
    }

    #[test]
    fn timeout_policy_fixed_requires_timeout_above_lock() {
        let mut offer = BoundlessConfig::default().offer_params.batch;
        offer.timeouts = super::TimeoutPolicy::Fixed {
            lock_timeout_secs: 600,
            timeout_secs: 600,
        };
        let err = validate_offer_spec(&offer).expect_err("fixed timeout not above lock");
        assert!(err.contains("timeout_secs must be greater than lock_timeout_secs"));
    }

    #[test]
    fn timeout_policy_fixed_deserializes_from_tagged_table() {
        let offer: BoundlessOfferParams = serde_json::from_value(serde_json::json!({
            "pricing_mode": "manual",
            "max_price_per_mcycle": "0.0000006",
            "min_price_per_mcycle": "0",
            "ramp_up_start_sec": 0,
            "ramp_up_period_sec": 180,
            "lock_collateral": "50",
            "timeouts": { "mode": "fixed", "lock_timeout_secs": 600, "timeout_secs": 3600 }
        }))
        .expect("deserialize fixed timeout policy");
        assert!(matches!(
            offer.timeouts,
            super::TimeoutPolicy::Fixed {
                lock_timeout_secs: 600,
                timeout_secs: 3600
            }
        ));
    }

    #[test]
    fn offer_params_reject_stale_offer_level_key() {
        // A stale offer-level key left over from the pre-cutover schema (here
        // `dynamic_pricing_timeout_modifier`, which now lives only inside `[timeouts]`) must fail
        // closed rather than boot with the value silently ignored.
        let err = serde_json::from_value::<BoundlessOfferParams>(serde_json::json!({
            "pricing_mode": "market",
            "ramp_up_start_sec": 0,
            "ramp_up_period_sec": 180,
            "lock_collateral": "50",
            "timeouts": { "mode": "fixed", "lock_timeout_secs": 600, "timeout_secs": 3600 },
            "dynamic_pricing_timeout_modifier": 2.0
        }))
        .expect_err("stale offer-level key should be rejected");
        assert!(err.to_string().contains("dynamic_pricing_timeout_modifier"));
    }

    #[test]
    fn timeout_policy_modifier_rejected_in_manual_mode() {
        let mut offer = BoundlessConfig::default().offer_params.batch; // pricing_mode = manual by default
        offer.timeouts = super::TimeoutPolicy::PerMcycle {
            lock_timeout_ms_per_mcycle: 300,
            timeout_ms_per_mcycle: 900,
            dynamic_pricing_timeout_modifier: Some(2.0),
        };
        let err = validate_offer_spec(&offer).expect_err("modifier in manual mode");
        assert!(err.contains("dynamic_pricing_timeout_modifier is only valid"));
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
        offer.timeouts = TimeoutPolicy::PerMcycle {
            lock_timeout_ms_per_mcycle: 300,
            timeout_ms_per_mcycle: 900,
            dynamic_pricing_timeout_modifier: Some(2.0),
        };

        validate_offer_spec(&offer).expect("valid market offer with timeout modifier");
    }

    #[test]
    fn validate_offer_spec_rejects_timeout_modifier_below_one() {
        let mut offer = BoundlessConfig::default().offer_params.batch;
        offer.pricing_mode = BoundlessPricingMode::Market;
        offer.max_price_per_mcycle = None;
        offer.min_price_per_mcycle = None;
        offer.timeouts = TimeoutPolicy::PerMcycle {
            lock_timeout_ms_per_mcycle: 300,
            timeout_ms_per_mcycle: 900,
            dynamic_pricing_timeout_modifier: Some(0.5),
        };

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

    #[test]
    fn validate_offer_spec_accepts_manual_ceiling_at_or_above_max_price() {
        let mut offer = BoundlessConfig::default().offer_params.batch;
        offer.absolute_max_price_per_mcycle = offer.max_price_per_mcycle.clone();
        validate_offer_spec(&offer).expect("ceiling equal to max price");

        offer.absolute_max_price_per_mcycle = Some("0.0000024".to_string());
        validate_offer_spec(&offer).expect("ceiling above max price");
    }

    #[test]
    fn validate_offer_spec_rejects_manual_ceiling_below_max_price() {
        let mut offer = BoundlessConfig::default().offer_params.batch;
        offer.absolute_max_price_per_mcycle = Some("0.0000001".to_string());

        let err = validate_offer_spec(&offer).expect_err("ceiling below max price");
        assert!(err.contains("cannot be below max_price_per_mcycle"));
    }

    #[test]
    fn validate_offer_spec_rejects_unparsable_manual_ceiling() {
        let mut offer = BoundlessConfig::default().offer_params.batch;
        offer.absolute_max_price_per_mcycle = Some("not-a-price".to_string());

        let err = validate_offer_spec(&offer).expect_err("unparsable ceiling");
        assert!(err.contains("Failed to parse absolute_max_price_per_mcycle"));
    }

    #[test]
    fn validate_offer_spec_accepts_market_pricing_with_absolute_ceiling() {
        let mut offer = BoundlessConfig::default().offer_params.batch;
        offer.pricing_mode = BoundlessPricingMode::Market;
        offer.max_price_per_mcycle = None;
        offer.min_price_per_mcycle = None;
        offer.absolute_max_price_per_mcycle = Some("0.000000060".to_string());

        validate_offer_spec(&offer).expect("valid market offer with absolute ceiling");
    }

    #[test]
    fn validate_offer_spec_rejects_market_pricing_with_both_cap_spellings() {
        let mut offer = BoundlessConfig::default().offer_params.batch;
        offer.pricing_mode = BoundlessPricingMode::Market;
        offer.min_price_per_mcycle = None;
        offer.max_price_per_mcycle = Some("0.000000060".to_string());
        offer.absolute_max_price_per_mcycle = Some("0.000000060".to_string());

        let err = validate_offer_spec(&offer).expect_err("both cap spellings");
        assert!(err.contains("set only one of"));
    }

    #[test]
    fn validate_offer_spec_rejects_zero_market_absolute_ceiling() {
        let mut offer = BoundlessConfig::default().offer_params.batch;
        offer.pricing_mode = BoundlessPricingMode::Market;
        offer.max_price_per_mcycle = None;
        offer.min_price_per_mcycle = None;
        offer.absolute_max_price_per_mcycle = Some("0".to_string());

        let err = validate_offer_spec(&offer).expect_err("zero absolute ceiling");
        assert!(err.contains("absolute_max_price_per_mcycle must be positive"));
    }

    #[test]
    fn transaction_config_validates_all_bounds() {
        let valid = BoundlessTransactionConfig {
            receipt_timeout_ms: 90_000,
            fee_bump_bps: 5_000,
            max_replacements: 4,
            max_fee_per_gas_wei: "1000000000".to_string(),
        };
        assert_eq!(
            valid.validate().expect("valid transaction config"),
            1_000_000_000
        );

        let mut invalid = valid.clone();
        invalid.receipt_timeout_ms = 0;
        assert!(invalid.validate().is_err());

        let mut invalid = valid.clone();
        invalid.fee_bump_bps = 0;
        assert!(invalid.validate().is_err());

        let mut invalid = valid.clone();
        invalid.fee_bump_bps = 999;
        assert!(invalid.validate().is_err());

        let mut invalid = valid.clone();
        invalid.receipt_timeout_ms = 90_001;
        assert!(invalid.validate().is_err());

        let mut invalid = valid.clone();
        invalid.max_replacements = 5;
        assert!(
            invalid
                .validate()
                .expect_err("default receipt timeout permits four replacements")
                .contains("max_replacements + 1")
        );

        let mut invalid = valid;
        invalid.max_fee_per_gas_wei = "not-a-number".to_string();
        assert!(invalid.validate().is_err());

        invalid.max_fee_per_gas_wei = "0".to_string();
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn transaction_config_defaults_bounded_retry_knobs() {
        let config: BoundlessTransactionConfig =
            serde_json::from_str(r#"{"max_fee_per_gas_wei":"1000000000"}"#)
                .expect("minimal transaction policy");

        assert_eq!(config.receipt_timeout_ms, 90_000);
        assert_eq!(config.fee_bump_bps, 5_000);
        assert_eq!(config.max_replacements, 4);
        assert!(config.validate().is_ok());
    }
}
