//! Exact arithmetic; classifications never depend on rounded display strings.
use anyhow::{Result, ensure};
use num_bigint::BigUint;
use num_traits::{One, Zero};

use crate::contracts::{ActualClassification, DirectMetricsV1, RatioV1};

pub const ARITHMETIC_POLICY: &str =
    "biguint-rational-v1;unzen-proposal-ceiling;display-half-even-6;classify-integers";

pub fn ceil_mul_ratio(
    value: &BigUint,
    numerator: &BigUint,
    denominator: &BigUint,
) -> Result<BigUint> {
    ensure!(!denominator.is_zero(), "zero denominator");
    let product = value * numerator;
    Ok(&product / denominator + BigUint::from(!(&product % denominator).is_zero()))
}

pub fn format_ratio_half_even(
    numerator: &BigUint,
    denominator: &BigUint,
    places: u32,
) -> Result<String> {
    ensure!(!denominator.is_zero(), "zero denominator");
    ensure!(places <= 100, "display precision exceeds 100 places");
    let scale = BigUint::from(10u32).pow(places);
    let scaled = numerator * &scale;
    let mut rounded = &scaled / denominator;
    let twice_remainder = (&scaled % denominator) * 2u32;
    if twice_remainder > *denominator
        || (twice_remainder == *denominator && (&rounded % 2u32).is_one())
    {
        rounded += 1u32;
    }
    if places == 0 {
        return Ok(rounded.to_string());
    }
    Ok(format!(
        "{}.{:0>width$}",
        &rounded / &scale,
        (&rounded % &scale).to_string(),
        width = places as usize
    ))
}

pub fn ratio(numerator: &BigUint, denominator: &BigUint) -> Result<RatioV1> {
    Ok(RatioV1 {
        numerator: numerator.to_string(),
        denominator: denominator.to_string(),
        display: format_ratio_half_even(numerator, denominator, 6)?,
    })
}

pub fn direct_metrics(prediction: &BigUint, actual: &BigUint) -> Result<DirectMetricsV1> {
    Ok(DirectMetricsV1 {
        classification: if actual.is_zero() {
            ActualClassification::ZeroActual
        } else if prediction.is_zero() {
            ActualClassification::InfiniteUnderpricing
        } else {
            ActualClassification::Positive
        },
        underquoted: prediction < actual,
        prediction_over_actual: (!actual.is_zero())
            .then(|| ratio(prediction, actual))
            .transpose()?,
        actual_over_prediction: (!prediction.is_zero())
            .then(|| ratio(actual, prediction))
            .transpose()?,
    })
}
