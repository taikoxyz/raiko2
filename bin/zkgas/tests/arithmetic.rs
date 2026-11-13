use num_bigint::BigUint;
use zkgas::{
    arithmetic::{ceil_mul_ratio, direct_metrics, format_ratio_half_even},
    contracts::ActualClassification,
};

fn n(text: &str) -> BigUint {
    text.parse().unwrap()
}

#[test]
fn unzen_rounds_the_proposal_sum_once_using_the_exact_sealed_scalar() {
    let numerator = n(
        "210937703648831249547430320136465120349542602443640317268060183142176006857255572450221059359429167",
    );
    let denominator = n(
        "2500000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
    );
    assert_eq!(
        ceil_mul_ratio(&n("2"), &numerator, &denominator).unwrap(),
        n("169")
    );
    assert_eq!(
        ceil_mul_ratio(&n("1"), &numerator, &denominator).unwrap() * 2u32,
        n("170")
    );
}

#[test]
fn arbitrary_precision_ceiling_and_zero_boundaries() {
    assert_eq!(
        ceil_mul_ratio(
            &n("340282366920938463463374607431768211456"),
            &n("3"),
            &n("2")
        )
        .unwrap(),
        n("510423550381407695195061911147652317184")
    );
    assert_eq!(ceil_mul_ratio(&n("99"), &n("0"), &n("2")).unwrap(), n("0"));
    assert!(ceil_mul_ratio(&n("0"), &n("0"), &n("0")).is_err());
    assert!(format_ratio_half_even(&n("1"), &n("0"), 6).is_err());
}

#[test]
fn display_rounds_exact_ties_half_even_in_both_directions() {
    for (numerator, denominator, expected) in [
        ("2469133", "2000000", "1.234566"),
        ("2469135", "2000000", "1.234568"),
        ("19999999", "2000000", "10.000000"),
        ("0", "1", "0.000000"),
    ] {
        assert_eq!(
            format_ratio_half_even(&n(numerator), &n(denominator), 6).unwrap(),
            expected
        );
    }
}

#[test]
fn classification_uses_unrounded_integers_and_distinguishes_zero_cases() {
    let metrics = direct_metrics(&n("10000000"), &n("10000001")).unwrap();
    assert!(metrics.underquoted);
    assert_eq!(metrics.prediction_over_actual.unwrap().display, "1.000000");
    let metrics = direct_metrics(&n("0"), &n("1")).unwrap();
    assert_eq!(
        metrics.classification,
        ActualClassification::InfiniteUnderpricing
    );
    assert!(metrics.underquoted);
    assert!(metrics.actual_over_prediction.is_none());
    assert_eq!(metrics.prediction_over_actual.unwrap().display, "0.000000");
    for prediction in ["0", "1"] {
        let metrics = direct_metrics(&n(prediction), &n("0")).unwrap();
        assert_eq!(metrics.classification, ActualClassification::ZeroActual);
        assert!(!metrics.underquoted);
        assert!(metrics.prediction_over_actual.is_none());
    }
}
