use std::collections::HashSet;
use std::sync::OnceLock;

use serde::Deserialize;

const EMBEDDED_MODEL: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../experiments/risc0-zkgas/models/risc0-zkgas-m2-v1.json"
));

static ESTIMATION_MODEL: OnceLock<Result<EstimationModel, String>> = OnceLock::new();

pub(crate) struct EstimationModel(ValidatedModelArtifact);

pub(crate) fn estimation_model() -> Result<&'static EstimationModel, String> {
    ESTIMATION_MODEL
        .get_or_init(|| parse_model(EMBEDDED_MODEL))
        .as_ref()
        .map_err(Clone::clone)
}

pub fn validate_estimation_model() -> Result<(), String> {
    estimation_model().map(|_| ())
}

fn parse_model(input: &str) -> Result<EstimationModel, String> {
    let artifact = serde_json::from_str(input)
        .map_err(|error| format!("invalid estimation model JSON: {error}"))?;
    validate_artifact(&artifact)?;
    Ok(EstimationModel(artifact))
}

fn validate_artifact(artifact: &ValidatedModelArtifact) -> Result<(), String> {
    if artifact.schema_version != 1 {
        return Err("unsupported estimation model schema_version".to_string());
    }
    if artifact.model_id != "risc0-zkgas-m2-v1" {
        return Err("unsupported estimation model_id".to_string());
    }
    if artifact.originating_experiment_model != "M2" {
        return Err("unsupported originating experiment model".to_string());
    }

    validate_release_provenance(&artifact.proposal.provenance, "proposal")?;
    validate_sha256(
        &artifact.proposal.raw_input_rows_sha256,
        "proposal raw input rows",
    )?;
    validate_sha256(
        &artifact.proposal.validation_fixture_sha256,
        "proposal validation fixture",
    )?;

    let scaled = &artifact.proposal.coefficients.scaled;
    if scaled.scale == 0
        || scaled.intercept == 0
        || scaled.total_zkgas == 0
        || scaled.block_count == 0
    {
        return Err("scaled proposal coefficients must be non-zero".to_string());
    }
    let decimal = &artifact.proposal.coefficients.decimal;
    if decimal.intercept.is_empty()
        || decimal.total_zkgas.is_empty()
        || decimal.block_count.is_empty()
    {
        return Err("decimal proposal coefficients must be present".to_string());
    }

    validate_domains(&artifact.proposal.domains)?;
    validate_cohorts(&artifact.proposal.cohorts)?;
    validate_aggregation(&artifact.aggregation)?;
    Ok(())
}

fn validate_release_provenance(provenance: &ReleaseProvenance, label: &str) -> Result<(), String> {
    if !is_hex(&provenance.source_revision, 40) {
        return Err(format!(
            "{label} source_revision must be a 40-character SHA-1"
        ));
    }
    validate_image_id(&provenance.image_id, label)?;
    validate_sha256(&provenance.elf_sha256, &format!("{label} ELF"))?;
    if !is_semver_triplet(&provenance.risc0_version) {
        return Err(format!("{label} risc0_version must be a semantic version"));
    }
    if provenance.execution_po2 == 0 {
        return Err(format!("{label} execution_po2 must be non-zero"));
    }
    Ok(())
}

fn validate_domains(domains: &[Domain]) -> Result<(), String> {
    if domains.len() != 2 {
        return Err("proposal domains must contain Hoodi and Mainnet exactly once".to_string());
    }

    let mut networks = HashSet::new();
    for domain in domains {
        if !matches!(domain.network.as_str(), "taiko_hoodi" | "taiko_mainnet") {
            return Err(format!("unsupported proposal domain {}", domain.network));
        }
        if !networks.insert(domain.network.as_str()) {
            return Err(format!("duplicate proposal domain {}", domain.network));
        }
        validate_range(&domain.block_count, "block_count")?;
        validate_range(&domain.total_zkgas, "total_zkgas")?;
    }
    if networks.len() != 2 {
        return Err("proposal domains must contain Hoodi and Mainnet".to_string());
    }
    Ok(())
}

fn validate_range(range: &InclusiveRange, label: &str) -> Result<(), String> {
    if range.minimum == 0 || range.minimum > range.maximum {
        return Err(format!("invalid {label} domain range"));
    }
    Ok(())
}

fn validate_cohorts(cohorts: &Cohorts) -> Result<(), String> {
    let hoodi = &cohorts.hoodi;
    if hoodi.fit_count == 0 || hoodi.calibration_count == 0 {
        return Err("Hoodi cohort counts must be non-zero".to_string());
    }
    validate_hoodi_diagnostics(&hoodi.continuous, hoodi.calibration_count, "continuous")?;
    validate_hoodi_diagnostics(
        &hoodi.scaled_integer,
        hoodi.calibration_count,
        "scaled integer",
    )?;

    let mainnet = &cohorts.mainnet;
    if mainnet.evaluation_count == 0
        || !mainnet.influenced_model_selection
        || mainnet.untouched_holdout
    {
        return Err("Mainnet evaluation provenance is inconsistent".to_string());
    }
    validate_mainnet_diagnostics(&mainnet.continuous, mainnet.evaluation_count, "continuous")?;
    validate_mainnet_diagnostics(
        &mainnet.scaled_integer,
        mainnet.evaluation_count,
        "scaled integer",
    )?;
    Ok(())
}

fn validate_hoodi_diagnostics(
    diagnostics: &HoodiDiagnostics,
    cohort_count: u32,
    label: &str,
) -> Result<(), String> {
    if diagnostics.underquote_count > cohort_count
        || diagnostics.over_ten_percent_count > cohort_count
        || diagnostics.mape_percent.is_empty()
        || diagnostics.max_absolute_error_percent.is_empty()
        || diagnostics.max_underquote_percent.is_empty()
    {
        return Err(format!("inconsistent Hoodi {label} diagnostics"));
    }
    Ok(())
}

fn validate_mainnet_diagnostics(
    diagnostics: &MainnetDiagnostics,
    cohort_count: u32,
    label: &str,
) -> Result<(), String> {
    if diagnostics.underquote_count > cohort_count
        || diagnostics.overquote_over_ten_percent_count > cohort_count
        || diagnostics
            .underquote_count
            .saturating_add(diagnostics.overquote_over_ten_percent_count)
            > cohort_count
        || diagnostics.mape_percent.is_empty()
        || diagnostics.max_underquote_percent.is_empty()
        || diagnostics.max_overquote_percent.is_empty()
    {
        return Err(format!("inconsistent Mainnet {label} diagnostics"));
    }
    Ok(())
}

fn validate_aggregation(aggregation: &Aggregation) -> Result<(), String> {
    if aggregation.per_child_mcycles == 0 {
        return Err("aggregation per_child_mcycles must be non-zero".to_string());
    }
    validate_aggregation_provenance(
        &aggregation.provenance,
        aggregation.measurements.is_empty() && aggregation.calibrated_counts.is_empty(),
    )?;

    let mut rows = HashSet::new();
    let mut enabled_counts = HashSet::new();
    for measurement in &aggregation.measurements {
        if !(1..=5).contains(&measurement.child_count) {
            return Err("aggregation child_count must be in 1..=5".to_string());
        }
        if !rows.insert(measurement.child_count) {
            return Err("duplicate aggregation child_count".to_string());
        }
        if measurement.actual_mcycles == 0 || measurement.predicted_mcycles == 0 {
            return Err("aggregation measured mcycles must be non-zero".to_string());
        }
        if measurement.enabled && !aggregation_measurement_is_accepted(measurement) {
            return Err(
                "enabled aggregation measurement exceeds accepted error budget".to_string(),
            );
        }
        if measurement.enabled {
            enabled_counts.insert(measurement.child_count);
        }
    }

    let mut calibrated_counts = HashSet::new();
    for &count in &aggregation.calibrated_counts {
        if !(1..=5).contains(&count) || !calibrated_counts.insert(count) {
            return Err("invalid or duplicate aggregation calibrated count".to_string());
        }
    }
    if calibrated_counts != enabled_counts {
        return Err(
            "aggregation calibrated counts must exactly match enabled measurements".to_string(),
        );
    }
    Ok(())
}

fn aggregation_measurement_is_accepted(measurement: &AggregationMeasurement) -> bool {
    let actual = u128::from(measurement.actual_mcycles);
    let predicted = u128::from(measurement.predicted_mcycles);
    let absolute_error = actual.abs_diff(predicted);
    let underquote = actual.saturating_sub(predicted);
    absolute_error * 100 <= actual * 10 && underquote * 100 <= actual * 10
}

fn validate_aggregation_provenance(
    provenance: &AggregationProvenance,
    is_uncalibrated: bool,
) -> Result<(), String> {
    match &provenance.image_id {
        Some(image_id) => validate_image_id(image_id, "aggregation")?,
        None if is_uncalibrated => {}
        None => {
            return Err(
                "aggregation image_id is required once calibration measurements exist".to_string(),
            );
        }
    }
    validate_sha256(&provenance.elf_sha256, "aggregation ELF")?;
    if provenance.execution_po2 == 0 {
        return Err("aggregation execution_po2 must be non-zero".to_string());
    }
    Ok(())
}

fn validate_sha256(value: &str, label: &str) -> Result<(), String> {
    if is_hex(value, 64) {
        Ok(())
    } else {
        Err(format!(
            "{label} SHA-256 must be 64 lowercase hexadecimal characters"
        ))
    }
}

fn validate_image_id(value: &str, label: &str) -> Result<(), String> {
    if value.starts_with("0x") && is_hex(&value[2..], 64) {
        Ok(())
    } else {
        Err(format!(
            "{label} image_id must be a 32-byte hexadecimal value"
        ))
    }
}

fn is_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte.is_ascii_lowercase())
}

fn is_semver_triplet(value: &str) -> bool {
    value.split('.').count() == 3
        && value.split('.').all(|component| {
            !component.is_empty() && component.bytes().all(|byte| byte.is_ascii_digit())
        })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ValidatedModelArtifact {
    schema_version: u32,
    model_id: String,
    originating_experiment_model: String,
    proposal: Proposal,
    aggregation: Aggregation,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Proposal {
    provenance: ReleaseProvenance,
    raw_input_rows_sha256: String,
    validation_fixture_sha256: String,
    coefficients: ProposalCoefficients,
    domains: Vec<Domain>,
    cohorts: Cohorts,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReleaseProvenance {
    source_revision: String,
    image_id: String,
    elf_sha256: String,
    risc0_version: String,
    execution_po2: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProposalCoefficients {
    decimal: DecimalCoefficients,
    scaled: ScaledCoefficients,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DecimalCoefficients {
    intercept: String,
    total_zkgas: String,
    block_count: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScaledCoefficients {
    scale: u64,
    intercept: u64,
    total_zkgas: u64,
    block_count: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Domain {
    network: String,
    block_count: InclusiveRange,
    total_zkgas: InclusiveRange,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InclusiveRange {
    minimum: u64,
    maximum: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Cohorts {
    hoodi: HoodiCohort,
    mainnet: MainnetCohort,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HoodiCohort {
    fit_count: u32,
    calibration_count: u32,
    continuous: HoodiDiagnostics,
    scaled_integer: HoodiDiagnostics,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HoodiDiagnostics {
    underquote_count: u32,
    mape_percent: String,
    max_absolute_error_percent: String,
    max_underquote_percent: String,
    over_ten_percent_count: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MainnetCohort {
    evaluation_count: u32,
    influenced_model_selection: bool,
    untouched_holdout: bool,
    continuous: MainnetDiagnostics,
    scaled_integer: MainnetDiagnostics,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MainnetDiagnostics {
    underquote_count: u32,
    mape_percent: String,
    max_underquote_percent: String,
    overquote_over_ten_percent_count: u32,
    max_overquote_percent: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Aggregation {
    per_child_mcycles: u64,
    provenance: AggregationProvenance,
    measurements: Vec<AggregationMeasurement>,
    calibrated_counts: Vec<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AggregationProvenance {
    image_id: Option<String>,
    elf_sha256: String,
    execution_po2: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AggregationMeasurement {
    child_count: u32,
    actual_mcycles: u64,
    predicted_mcycles: u64,
    enabled: bool,
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    fn valid_artifact() -> Value {
        json!({
            "schema_version": 1,
            "model_id": "risc0-zkgas-m2-v1",
            "originating_experiment_model": "M2",
            "proposal": {
                "provenance": {
                    "source_revision": "4f8300497aba75605b9b8568b1955faa1f7f04bc",
                    "image_id": "0xd6ab71c22201c23ef512b706f2e2d720f6da1b559fb76834aa9d4e35276f6e10",
                    "elf_sha256": "d7a4aca3769005d30772a6a1d4c47c95f7d6692244a3b017b181935a855e6b35",
                    "risc0_version": "3.0.5",
                    "execution_po2": 20
                },
                "raw_input_rows_sha256": "be824f1262862525aaa961e568feb1e7b911031256b7ddf1d3f7ef6b5236e18c",
                "validation_fixture_sha256": "dff36c84683011825a7372e43f846b678266f0f062515f44631922e9a7c47767",
                "coefficients": {
                    "decimal": {"intercept": "511.8367085993759", "total_zkgas": "0.000003714503729246405", "block_count": "2.2737130481392764"},
                    "scaled": {"scale": 1000000000000u64, "intercept": 511836708599376u64, "total_zkgas": 3714504u64, "block_count": 2273713048139u64}
                },
                "domains": [
                    {"network": "taiko_hoodi", "block_count": {"minimum": 155, "maximum": 192}, "total_zkgas": {"minimum": 369558586, "maximum": 459162040}},
                    {"network": "taiko_mainnet", "block_count": {"minimum": 184, "maximum": 192}, "total_zkgas": {"minimum": 216314230, "maximum": 310638954}}
                ],
                "cohorts": {
                    "hoodi": {"fit_count": 80, "calibration_count": 40, "continuous": {"underquote_count": 17, "mape_percent": "0.094557", "max_absolute_error_percent": "0.279512", "max_underquote_percent": "0.279512", "over_ten_percent_count": 0}, "scaled_integer": {"underquote_count": 12, "mape_percent": "0.093492", "max_absolute_error_percent": "0.264550", "max_underquote_percent": "0.264550", "over_ten_percent_count": 0}},
                    "mainnet": {"evaluation_count": 20, "influenced_model_selection": true, "untouched_holdout": false, "continuous": {"underquote_count": 19, "mape_percent": "5.87", "max_underquote_percent": "5.75", "overquote_over_ten_percent_count": 1, "max_overquote_percent": "21.94"}, "scaled_integer": {"underquote_count": 19, "mape_percent": "5.8422", "max_underquote_percent": "5.7234", "overquote_over_ten_percent_count": 1, "max_overquote_percent": "21.9679"}}
                }
            },
            "aggregation": {
                "per_child_mcycles": 180,
                "provenance": {"image_id": null, "elf_sha256": "fd56481a38855c3d85488cc267653ae390633c16ba1612fcf2d4891f5b30d924", "execution_po2": 20},
                "measurements": [],
                "calibrated_counts": []
            }
        })
    }

    fn parse(value: Value) -> Result<super::EstimationModel, String> {
        super::parse_model(&value.to_string())
    }

    #[test]
    fn model_parses_valid_minimal_artifact() {
        parse(valid_artifact()).expect("valid artifact");
    }

    #[test]
    fn model_rejects_malformed_json() {
        assert!(super::parse_model("{").is_err());
    }

    #[test]
    fn model_rejects_missing_fields() {
        let mut artifact = valid_artifact();
        artifact.as_object_mut().unwrap().remove("proposal");
        assert!(parse(artifact).is_err());
    }

    #[test]
    fn model_rejects_unknown_fields() {
        let mut artifact = valid_artifact();
        artifact["unexpected"] = json!(true);
        assert!(parse(artifact).is_err());
    }

    #[test]
    fn model_rejects_wrong_schema_or_model_ids() {
        let mut artifact = valid_artifact();
        artifact["schema_version"] = json!(2);
        assert!(parse(artifact).is_err());
        let mut artifact = valid_artifact();
        artifact["model_id"] = json!("other-model");
        assert!(parse(artifact).is_err());
    }

    #[test]
    fn model_rejects_invalid_hashes() {
        let mut artifact = valid_artifact();
        artifact["proposal"]["provenance"]["elf_sha256"] = json!("not-a-sha256");
        assert!(parse(artifact).is_err());

        let mut artifact = valid_artifact();
        artifact["proposal"]["provenance"]["image_id"] = json!("0xnot-an-image");
        assert!(parse(artifact).is_err());
    }

    #[test]
    fn model_rejects_inverted_domains() {
        let mut artifact = valid_artifact();
        artifact["proposal"]["domains"][0]["block_count"]["minimum"] = json!(193);
        assert!(parse(artifact).is_err());
    }

    #[test]
    fn model_rejects_zero_scaled_coefficients() {
        let mut artifact = valid_artifact();
        artifact["proposal"]["coefficients"]["scaled"]["scale"] = json!(0);
        assert!(parse(artifact).is_err());
    }

    #[test]
    fn model_rejects_inconsistent_aggregation_calibration_rows() {
        let mut artifact = valid_artifact();
        artifact["aggregation"]["measurements"] = json!([
            {"child_count": 1, "actual_mcycles": 180, "predicted_mcycles": 180, "enabled": true}
        ]);
        assert!(parse(artifact).is_err());
    }

    #[test]
    fn model_requires_an_aggregation_image_id_once_measurements_exist() {
        let mut artifact = valid_artifact();
        artifact["aggregation"]["measurements"] = json!([
            {"child_count": 1, "actual_mcycles": 180, "predicted_mcycles": 180, "enabled": false}
        ]);
        assert!(parse(artifact).is_err());
    }

    #[test]
    fn model_uses_production_parameters_from_the_artifact() {
        let mut artifact = valid_artifact();
        artifact["proposal"]["coefficients"]["scaled"]["intercept"] = json!(42);
        let model = parse(artifact).expect("artifact with changed intercept");
        assert_eq!(model.0.proposal.coefficients.scaled.intercept, 42);
    }

    #[test]
    fn model_embedded_artifact_validates() {
        super::validate_estimation_model().expect("embedded artifact validates");
    }
}
