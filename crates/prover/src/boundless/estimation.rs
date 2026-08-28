use std::collections::HashSet;
use std::sync::OnceLock;

use raiko2_primitives::{
    RaikoError, RaikoResult,
    chain_spec::{ForkId, TaikoFork},
};
use raiko2_primitives_shasta::{GuestInput, ShastaRisc0AggregationGuestInput};
use serde::Deserialize;

use crate::{validated_shasta_proposal_input, validated_shasta_zk_aggregation_output};

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

/// Validate the embedded quote-estimation model artifact.
///
/// # Errors
///
/// Returns an error when the embedded JSON is malformed, unsupported, or internally inconsistent.
pub fn validate_estimation_model() -> Result<(), String> {
    estimation_model().map(|_| ())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EstimateUnavailable {
    ExecutionPo2,
    Fork,
    Chain,
    Domain,
    ZeroZkGas,
    Numeric,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EstimatedRequestMetadata {
    pub model_id: String,
    pub mcycles: u32,
    pub journal: Vec<u8>,
}

pub(crate) fn estimate_proposal(
    input: &GuestInput,
    execution_po2: u32,
) -> RaikoResult<Result<EstimatedRequestMetadata, EstimateUnavailable>> {
    let first_witness = input.witnesses.first().ok_or_else(|| {
        RaikoError::InvalidRequestConfig(
            "cannot estimate Boundless proposal without witnesses".to_string(),
        )
    })?;
    let journal = validated_shasta_proposal_input(&input.proof_carry_data)?
        .as_slice()
        .to_vec();
    let model = estimation_model().map_err(|error| {
        RaikoError::InvalidRequestConfig(format!("invalid Boundless estimation model: {error}"))
    })?;
    let proposal = &model.0.proposal;

    if execution_po2 != proposal.provenance.execution_po2 {
        return Ok(Err(EstimateUnavailable::ExecutionPo2));
    }
    if !proposal_estimation_available(input) {
        return Ok(Err(EstimateUnavailable::Fork));
    }

    let network = first_witness.chain_spec.name.as_str();
    if input
        .witnesses
        .iter()
        .any(|witness| witness.chain_spec.name != network)
    {
        return Ok(Err(EstimateUnavailable::Chain));
    }
    let Some(domain) = proposal
        .domains
        .iter()
        .find(|domain| domain.network == network)
    else {
        return Ok(Err(EstimateUnavailable::Chain));
    };

    let block_count =
        u128::try_from(input.witnesses.len()).map_err(|_| EstimateUnavailable::Numeric);
    let block_count = match block_count {
        Ok(block_count) => block_count,
        Err(unavailable) => return Ok(Err(unavailable)),
    };
    let mut total_zkgas = 0_u128;
    for witness in &input.witnesses {
        let Ok(difficulty) = u128::try_from(witness.block.header.difficulty) else {
            return Ok(Err(EstimateUnavailable::Numeric));
        };
        total_zkgas = match total_zkgas.checked_add(difficulty) {
            Some(total_zkgas) => total_zkgas,
            None => return Ok(Err(EstimateUnavailable::Numeric)),
        };
    }
    if total_zkgas == 0 {
        return Ok(Err(EstimateUnavailable::ZeroZkGas));
    }
    if !domain.block_count.contains(block_count) || !domain.total_zkgas.contains(total_zkgas) {
        return Ok(Err(EstimateUnavailable::Domain));
    }

    let mcycles = match estimate_mcycles(&proposal.coefficients.scaled, total_zkgas, block_count) {
        Ok(mcycles) => mcycles,
        Err(unavailable) => return Ok(Err(unavailable)),
    };
    Ok(Ok(EstimatedRequestMetadata {
        model_id: model.0.model_id.clone(),
        mcycles,
        journal,
    }))
}

pub(crate) fn estimate_aggregation(
    encoded_input: &[u8],
) -> RaikoResult<Result<EstimatedRequestMetadata, EstimateUnavailable>> {
    let model = estimation_model().map_err(|error| {
        RaikoError::InvalidRequestConfig(format!("invalid Boundless estimation model: {error}"))
    })?;
    estimate_aggregation_with_model(encoded_input, model)
}

fn estimate_aggregation_with_model(
    encoded_input: &[u8],
    model: &EstimationModel,
) -> RaikoResult<Result<EstimatedRequestMetadata, EstimateUnavailable>> {
    let input: ShastaRisc0AggregationGuestInput =
        bincode::deserialize(encoded_input).map_err(|error| {
            RaikoError::InvalidRequestConfig(format!(
                "failed to decode Boundless aggregation input: {error}"
            ))
        })?;
    if input.receipts.len() != input.proof_carry_data_vec.len() {
        return Err(RaikoError::InvalidRequestConfig(format!(
            "aggregation receipt/proof carry count mismatch: {} vs {}",
            input.receipts.len(),
            input.proof_carry_data_vec.len()
        )));
    }

    let Ok(child_count) = u32::try_from(input.receipts.len()) else {
        return Ok(Err(EstimateUnavailable::Numeric));
    };
    let journal = validated_shasta_zk_aggregation_output(
        input.image_id,
        input.proof_carry_data_vec,
        input.prover_address,
    )?
    .as_slice()
    .to_vec();

    let aggregation = &model.0.aggregation;
    if !aggregation.calibrated_counts.contains(&child_count) {
        return Ok(Err(EstimateUnavailable::Domain));
    }
    let Some(mcycles) = aggregation
        .per_child_mcycles
        .checked_mul(u64::from(child_count))
        .and_then(|mcycles| u32::try_from(mcycles).ok())
    else {
        return Ok(Err(EstimateUnavailable::Numeric));
    };

    Ok(Ok(EstimatedRequestMetadata {
        model_id: model.0.model_id.clone(),
        mcycles,
        journal,
    }))
}

fn estimate_mcycles(
    coefficients: &ScaledCoefficients,
    total_zkgas: u128,
    block_count: u128,
) -> Result<u32, EstimateUnavailable> {
    let scale = u128::from(coefficients.scale);
    if scale == 0 {
        return Err(EstimateUnavailable::Numeric);
    }
    let zkgas_term = total_zkgas
        .checked_mul(u128::from(coefficients.total_zkgas))
        .ok_or(EstimateUnavailable::Numeric)?;
    let block_count_term = block_count
        .checked_mul(u128::from(coefficients.block_count))
        .ok_or(EstimateUnavailable::Numeric)?;
    let numerator = u128::from(coefficients.intercept)
        .checked_add(zkgas_term)
        .and_then(|value| value.checked_add(block_count_term))
        .ok_or(EstimateUnavailable::Numeric)?;
    let mcycles = (numerator / scale)
        .checked_add(u128::from(!numerator.is_multiple_of(scale)))
        .ok_or(EstimateUnavailable::Numeric)?;
    let mcycles = u32::try_from(mcycles).map_err(|_| EstimateUnavailable::Numeric)?;
    if mcycles == 0 {
        return Err(EstimateUnavailable::Numeric);
    }
    Ok(mcycles)
}

impl InclusiveRange {
    fn contains(&self, value: u128) -> bool {
        (u128::from(self.minimum)..=u128::from(self.maximum)).contains(&value)
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum ActiveTaikoForkRank {
    Hekla,
    Ontake,
    Pacaya,
    Shasta,
    Unzen,
    #[cfg(test)]
    Future,
}

const fn active_taiko_fork_rank(fork: TaikoFork) -> ActiveTaikoForkRank {
    match fork {
        TaikoFork::Hekla => ActiveTaikoForkRank::Hekla,
        TaikoFork::Ontake => ActiveTaikoForkRank::Ontake,
        TaikoFork::Pacaya => ActiveTaikoForkRank::Pacaya,
        TaikoFork::Shasta => ActiveTaikoForkRank::Shasta,
        TaikoFork::Unzen => ActiveTaikoForkRank::Unzen,
    }
}

const fn supported_active_taiko_fork(highest: Option<ActiveTaikoForkRank>) -> bool {
    matches!(highest, Some(ActiveTaikoForkRank::Unzen))
}

fn proposal_estimation_available(input: &GuestInput) -> bool {
    input.witnesses.iter().all(|witness| {
        let highest = witness
            .chain_spec
            .hard_forks
            .iter()
            .filter_map(|(fork_id, condition)| match fork_id {
                ForkId::Taiko(fork)
                    if condition
                        .active(witness.block.header.number, witness.block.header.timestamp) =>
                {
                    Some(active_taiko_fork_rank(*fork))
                }
                ForkId::Standard(_) | ForkId::Taiko(_) => None,
            })
            .max();
        supported_active_taiko_fork(highest)
    })
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
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
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
    use std::{collections::HashSet, str::FromStr};

    use alloy_primitives::{Address, B256, U256, Uint, address, b256};
    use raiko2_guest_common::aggregate_shasta_zk_with_verifier;
    use raiko2_primitives::{
        ChainSpec, StatelessInput,
        chain_spec::{ForkCondition, ForkId, TaikoFork},
    };
    use raiko2_primitives_shasta::{
        GuestInput, ShastaRisc0AggregationGuestInput, ShastaZkAggregationGuestInput,
        instance::words_to_bytes_le,
    };
    use raiko2_protocol_shasta::libhash::hash_shasta_subproof_input;
    use raiko2_protocol_shasta::shasta::{
        Checkpoint, ProofCarryData, ShastaTransitionInput, TransitionInputData,
    };
    use serde::Deserialize;
    use serde_json::{Value, json};
    use sha2::{Digest, Sha256};

    use super::{
        ActiveTaikoForkRank, EstimateUnavailable, ScaledCoefficients, estimate_aggregation,
        estimate_aggregation_with_model, estimate_mcycles, estimate_proposal,
        supported_active_taiko_fork,
    };

    fn proposal_input(network: &str, block_count: usize, total_zkgas: u64) -> GuestInput {
        let mut chain_spec = ChainSpec {
            name: network.to_string(),
            ..Default::default()
        };
        chain_spec
            .hard_forks
            .insert(ForkId::Taiko(TaikoFork::Unzen), ForkCondition::Block(0));

        let mut input = GuestInput::default();
        input.witnesses = (0..block_count)
            .map(|index| {
                let mut witness = StatelessInput {
                    chain_spec: chain_spec.clone(),
                    ..Default::default()
                };
                witness.block.header.number = u64::try_from(index).expect("test block number");
                witness.block.header.timestamp =
                    u64::try_from(index).expect("test block timestamp");
                witness
            })
            .collect();
        if let Some(first) = input.witnesses.first_mut() {
            first.block.header.difficulty = U256::from(total_zkgas);
        }
        input
    }

    fn proposal_result(
        input: &GuestInput,
    ) -> Result<super::EstimatedRequestMetadata, EstimateUnavailable> {
        estimate_proposal(input, 20).expect("structurally valid proposal input")
    }

    fn aggregation_carries() -> Vec<ProofCarryData> {
        let first_hash = b256!("1111111111111111111111111111111111111111111111111111111111111111");
        let first_checkpoint_hash =
            b256!("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
        vec![
            ProofCarryData {
                chain_id: 167_000,
                verifier: address!("00000000000000000000000000000000000000aa"),
                transition_input: TransitionInputData {
                    proposal_id: 1,
                    proposal_hash: first_hash,
                    parent_proposal_hash: B256::ZERO,
                    parent_block_hash: b256!(
                        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    ),
                    actual_prover: address!("00000000000000000000000000000000000000bb"),
                    transition: ShastaTransitionInput {
                        proposer: Address::ZERO,
                        timestamp: 101,
                    },
                    checkpoint: Checkpoint {
                        blockNumber: Uint::from(10_u64),
                        blockHash: first_checkpoint_hash,
                        stateRoot: b256!(
                            "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
                        ),
                    },
                },
            },
            ProofCarryData {
                chain_id: 167_000,
                verifier: address!("00000000000000000000000000000000000000aa"),
                transition_input: TransitionInputData {
                    proposal_id: 2,
                    proposal_hash: b256!(
                        "2222222222222222222222222222222222222222222222222222222222222222"
                    ),
                    parent_proposal_hash: first_hash,
                    parent_block_hash: first_checkpoint_hash,
                    actual_prover: address!("00000000000000000000000000000000000000bb"),
                    transition: ShastaTransitionInput {
                        proposer: Address::ZERO,
                        timestamp: 102,
                    },
                    checkpoint: Checkpoint {
                        blockNumber: Uint::from(11_u64),
                        blockHash: b256!(
                            "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
                        ),
                        stateRoot: b256!(
                            "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
                        ),
                    },
                },
            },
        ]
    }

    fn encoded_aggregation(
        carries: Vec<ProofCarryData>,
        receipts: Vec<Vec<u8>>,
        image_id: [u32; 8],
        prover_address: Address,
    ) -> Vec<u8> {
        bincode::serialize(&ShastaRisc0AggregationGuestInput {
            image_id,
            proof_carry_data_vec: carries,
            receipts,
            prover_address,
        })
        .expect("encode aggregation fixture")
    }

    fn aggregation_model(
        calibrated_counts: &[u32],
        per_child_mcycles: u64,
    ) -> super::EstimationModel {
        let mut artifact = valid_artifact();
        artifact["aggregation"]["per_child_mcycles"] = json!(per_child_mcycles);
        artifact["aggregation"]["provenance"]["image_id"] =
            json!("0xd6ab71c22201c23ef512b706f2e2d720f6da1b559fb76834aa9d4e35276f6e10");
        artifact["aggregation"]["measurements"] = Value::Array(
            calibrated_counts
                .iter()
                .map(|&child_count| {
                    json!({
                        "child_count": child_count,
                        "actual_mcycles": 1,
                        "predicted_mcycles": 1,
                        "enabled": true
                    })
                })
                .collect(),
        );
        artifact["aggregation"]["calibrated_counts"] = json!(calibrated_counts);
        parse(artifact).expect("valid calibrated aggregation model")
    }

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
    fn model_rejects_correct_length_non_hex_sha256() {
        let mut artifact = valid_artifact();
        artifact["proposal"]["provenance"]["elf_sha256"] = json!("g".repeat(64));
        assert!(parse(artifact).is_err());
    }

    #[test]
    fn model_rejects_correct_length_non_hex_image_id() {
        let mut artifact = valid_artifact();
        artifact["proposal"]["provenance"]["image_id"] = json!(format!("0x{}", "g".repeat(64)));
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

    #[test]
    fn proposal_empty_witnesses_are_a_direct_error() {
        assert!(estimate_proposal(&GuestInput::default(), 20).is_err());
    }

    #[test]
    fn proposal_malformed_carry_is_a_direct_error_before_hashing() {
        let mut input = proposal_input("taiko_hoodi", 155, 369_558_586);
        input.proof_carry_data.transition_input.transition.timestamp = 1_u64 << 48;

        assert!(estimate_proposal(&input, 20).is_err());
    }

    #[test]
    fn proposal_valid_carry_produces_the_exact_subproof_journal() {
        let input = proposal_input("taiko_hoodi", 155, 369_558_586);
        let expected = hash_shasta_subproof_input(&input.proof_carry_data);

        let estimate = proposal_result(&input).expect("proposal estimate available");

        assert_eq!(estimate.journal.len(), 32);
        assert_eq!(estimate.journal, expected.as_slice());
        assert_eq!(estimate.model_id, "risc0-zkgas-m2-v1");
    }

    #[test]
    fn proposal_hoodi_domain_accepts_its_lower_and_upper_boundaries() {
        let lower = proposal_input("taiko_hoodi", 155, 369_558_586);
        let upper = proposal_input("taiko_hoodi", 192, 459_162_040);

        assert_eq!(proposal_result(&lower).unwrap().mcycles, 2_237);
        assert!(proposal_result(&upper).is_ok());
    }

    #[test]
    fn proposal_mainnet_domain_accepts_its_lower_and_upper_boundaries() {
        let lower = proposal_input("taiko_mainnet", 184, 216_314_230);
        let upper = proposal_input("taiko_mainnet", 192, 310_638_954);

        assert!(proposal_result(&lower).is_ok());
        assert!(proposal_result(&upper).is_ok());
    }

    #[test]
    fn proposal_chain_domains_do_not_admit_the_other_chain_rectangle() {
        let hoodi_with_mainnet_zkgas = proposal_input("taiko_hoodi", 155, 216_314_230);
        let mainnet_with_hoodi_zkgas = proposal_input("taiko_mainnet", 184, 369_558_586);

        assert_eq!(
            proposal_result(&hoodi_with_mainnet_zkgas),
            Err(EstimateUnavailable::Domain)
        );
        assert_eq!(
            proposal_result(&mainnet_with_hoodi_zkgas),
            Err(EstimateUnavailable::Domain)
        );
    }

    #[test]
    fn proposal_mainnet_isolated_high_zkgas_sample_is_unavailable() {
        let input = proposal_input("taiko_mainnet", 192, 562_107_601);

        assert_eq!(proposal_result(&input), Err(EstimateUnavailable::Domain));
    }

    #[test]
    fn proposal_unknown_or_inconsistent_chain_is_unavailable() {
        let unknown = proposal_input("taiko_unknown", 184, 216_314_230);
        let mut inconsistent = proposal_input("taiko_mainnet", 184, 216_314_230);
        inconsistent.witnesses[183].chain_spec.name = "taiko_hoodi".to_string();

        assert_eq!(proposal_result(&unknown), Err(EstimateUnavailable::Chain));
        assert_eq!(
            proposal_result(&inconsistent),
            Err(EstimateUnavailable::Chain)
        );
    }

    #[test]
    fn proposal_execution_po2_must_match_the_artifact() {
        let input = proposal_input("taiko_hoodi", 155, 369_558_586);

        assert_eq!(
            estimate_proposal(&input, 19).unwrap(),
            Err(EstimateUnavailable::ExecutionPo2)
        );
    }

    #[test]
    fn proposal_every_witness_must_have_unzen_as_the_highest_active_taiko_fork() {
        let mut input = proposal_input("taiko_hoodi", 155, 369_558_586);
        let last = input.witnesses.last_mut().unwrap();
        last.chain_spec.hard_forks.clear();
        last.chain_spec
            .hard_forks
            .insert(ForkId::Taiko(TaikoFork::Shasta), ForkCondition::Block(0));

        assert_eq!(proposal_result(&input), Err(EstimateUnavailable::Fork));
    }

    #[test]
    fn proposal_ordered_fork_classifier_rejects_a_synthetic_post_unzen_rank() {
        assert!(supported_active_taiko_fork(Some(
            ActiveTaikoForkRank::Unzen
        )));
        assert!(!supported_active_taiko_fork(Some(
            ActiveTaikoForkRank::Future
        )));
    }

    #[test]
    fn proposal_zero_zkgas_is_unavailable() {
        let input = proposal_input("taiko_hoodi", 155, 0);

        assert_eq!(proposal_result(&input), Err(EstimateUnavailable::ZeroZkGas));
    }

    #[test]
    fn proposal_zkgas_checked_add_overflow_is_unavailable() {
        let mut input = proposal_input("taiko_hoodi", 155, 0);
        input.witnesses[0].block.header.difficulty = U256::from(u128::MAX);
        input.witnesses[1].block.header.difficulty = U256::from(1_u64);

        assert_eq!(proposal_result(&input), Err(EstimateUnavailable::Numeric));
    }

    #[test]
    fn proposal_formula_checked_add_and_multiply_overflow_are_unavailable() {
        let add_overflow = ScaledCoefficients {
            scale: 1,
            intercept: 1,
            total_zkgas: 1,
            block_count: 1,
        };
        let multiply_overflow = ScaledCoefficients {
            scale: 1,
            intercept: 1,
            total_zkgas: u64::MAX,
            block_count: 1,
        };

        assert_eq!(
            estimate_mcycles(&add_overflow, u128::MAX, 1),
            Err(EstimateUnavailable::Numeric)
        );
        assert_eq!(
            estimate_mcycles(&multiply_overflow, u128::MAX, 1),
            Err(EstimateUnavailable::Numeric)
        );
    }

    #[test]
    fn proposal_formula_final_u32_conversion_overflow_is_unavailable() {
        let coefficients = ScaledCoefficients {
            scale: 1,
            intercept: u64::from(u32::MAX) + 1,
            total_zkgas: 1,
            block_count: 1,
        };

        assert_eq!(
            estimate_mcycles(&coefficients, 1, 1),
            Err(EstimateUnavailable::Numeric)
        );
    }

    #[test]
    fn aggregation_malformed_bincode_is_a_direct_error() {
        assert!(estimate_aggregation(&[0xff, 0x00]).is_err());
    }

    #[test]
    fn aggregation_zero_children_is_a_direct_error() {
        let encoded = encoded_aggregation(vec![], vec![], [0; 8], Address::ZERO);

        assert!(estimate_aggregation(&encoded).is_err());
    }

    #[test]
    fn aggregation_receipt_carry_count_mismatch_is_a_direct_error() {
        let encoded = encoded_aggregation(
            vec![aggregation_carries().remove(0)],
            vec![],
            [0; 8],
            Address::ZERO,
        );

        assert!(estimate_aggregation(&encoded).is_err());
    }

    #[test]
    fn aggregation_nonzero_prover_address_is_a_direct_error() {
        let encoded = encoded_aggregation(
            vec![aggregation_carries().remove(0)],
            vec![vec![0xff]],
            [0; 8],
            address!("0000000000000000000000000000000000000001"),
        );

        assert!(estimate_aggregation(&encoded).is_err());
    }

    #[test]
    fn aggregation_invalid_uint48_is_a_direct_error_without_panicking() {
        let mut invalid_timestamp = aggregation_carries().remove(0);
        invalid_timestamp.transition_input.transition.timestamp = 1_u64 << 48;
        let mut invalid_proposal_id = aggregation_carries().remove(0);
        invalid_proposal_id.transition_input.proposal_id = 1_u64 << 48;

        for carry in [invalid_timestamp, invalid_proposal_id] {
            let encoded = encoded_aggregation(vec![carry], vec![vec![0xff]], [0; 8], Address::ZERO);
            let result = std::panic::catch_unwind(|| estimate_aggregation(&encoded));

            assert!(result.is_ok(), "invalid uint48 must not panic");
            assert!(result.expect("checked above").is_err());
        }
    }

    #[test]
    fn aggregation_invalid_carry_sequence_is_a_direct_error() {
        let mut carries = aggregation_carries();
        carries[1].transition_input.proposal_id = 3;
        let encoded =
            encoded_aggregation(carries, vec![vec![0xff], vec![0xfe]], [0; 8], Address::ZERO);

        assert!(estimate_aggregation(&encoded).is_err());
    }

    #[test]
    fn aggregation_invalid_carry_linkage_is_a_direct_error() {
        let mut carries = aggregation_carries();
        carries[1].transition_input.parent_proposal_hash = B256::repeat_byte(0x77);
        let encoded =
            encoded_aggregation(carries, vec![vec![0xff], vec![0xfe]], [0; 8], Address::ZERO);

        assert!(estimate_aggregation(&encoded).is_err());
    }

    #[test]
    fn aggregation_valid_input_matches_shared_output_with_little_endian_image_words() {
        let carries = aggregation_carries();
        let image_id = [
            0x0102_0304,
            0x1112_1314,
            0x2122_2324,
            0x3132_3334,
            0x4142_4344,
            0x5152_5354,
            0x6162_6364,
            0x7172_7374,
        ];
        let block_inputs = carries.iter().map(hash_shasta_subproof_input).collect();
        let zk_input = ShastaZkAggregationGuestInput {
            image_id,
            block_inputs,
            proof_carry_data_vec: carries.clone(),
            prover_address: Address::ZERO,
        };
        let image_id_b256 = B256::from(words_to_bytes_le(&image_id));
        let expected =
            aggregate_shasta_zk_with_verifier(&zk_input, image_id_b256, |_index, _input| Ok(()))
                .expect("shared aggregation output");
        let encoded = encoded_aggregation(
            carries,
            // Deliberately not valid receipt encodings: journal derivation must not inspect them.
            vec![vec![0xff], vec![0xfe]],
            image_id,
            Address::ZERO,
        );
        let model = aggregation_model(&[2], 180);

        let estimate = estimate_aggregation_with_model(&encoded, &model)
            .expect("structurally valid aggregation")
            .expect("calibrated aggregation count");

        assert_eq!(estimate.journal, expected.as_slice());
        assert_eq!(estimate.journal.len(), 32);
        assert_eq!(estimate.mcycles, 360);
        assert_eq!(estimate.model_id, "risc0-zkgas-m2-v1");
    }

    #[test]
    fn aggregation_embedded_empty_calibrated_set_fails_closed() {
        let encoded = encoded_aggregation(
            vec![aggregation_carries().remove(0)],
            vec![vec![0xff]],
            [0; 8],
            Address::ZERO,
        );

        assert_eq!(
            estimate_aggregation(&encoded).expect("structurally valid aggregation"),
            Err(EstimateUnavailable::Domain)
        );
    }

    #[test]
    fn aggregation_uncalibrated_count_is_unavailable() {
        let encoded = encoded_aggregation(
            aggregation_carries(),
            vec![vec![0xff], vec![0xfe]],
            [0; 8],
            Address::ZERO,
        );
        let model = aggregation_model(&[1], 180);

        assert_eq!(
            estimate_aggregation_with_model(&encoded, &model)
                .expect("structurally valid aggregation"),
            Err(EstimateUnavailable::Domain)
        );
    }

    #[test]
    fn aggregation_checked_multiply_overflow_is_unavailable() {
        let encoded = encoded_aggregation(
            aggregation_carries(),
            vec![vec![0xff], vec![0xfe]],
            [0; 8],
            Address::ZERO,
        );
        let model = aggregation_model(&[2], u64::MAX);

        assert_eq!(
            estimate_aggregation_with_model(&encoded, &model)
                .expect("structurally valid aggregation"),
            Err(EstimateUnavailable::Numeric)
        );
    }

    #[test]
    fn aggregation_final_u32_conversion_overflow_is_unavailable() {
        let encoded = encoded_aggregation(
            aggregation_carries(),
            vec![vec![0xff], vec![0xfe]],
            [0; 8],
            Address::ZERO,
        );
        let model = aggregation_model(&[2], u64::from(u32::MAX));

        assert_eq!(
            estimate_aggregation_with_model(&encoded, &model)
                .expect("structurally valid aggregation"),
            Err(EstimateUnavailable::Numeric)
        );
    }

    #[test]
    fn fixture_contract_rejects_untracked_rows() {
        let artifact = fixture_artifact();
        let rows = validation_fixture_rows();

        assert_eq!(
            hex::encode(Sha256::digest(VALIDATION_FIXTURE.as_bytes())),
            "dff36c84683011825a7372e43f846b678266f0f062515f44631922e9a7c47767"
        );
        assert_eq!(
            artifact.proposal.validation_fixture_sha256,
            "dff36c84683011825a7372e43f846b678266f0f062515f44631922e9a7c47767"
        );
        assert_eq!(rows.len(), 60);
        assert_eq!(
            rows.iter()
                .filter(|row| row.network == "taiko_hoodi" && row.split == "calibration")
                .count(),
            40
        );
        assert_eq!(
            rows.iter()
                .filter(|row| row.network == "taiko_mainnet" && row.split == "evaluation")
                .count(),
            20
        );
    }

    #[test]
    fn fixture_diagnostics_reproduce_committed_model() {
        let artifact = fixture_artifact();
        let rows = validation_fixture_rows();
        let coefficients = &artifact.proposal.coefficients;
        let decimal = DecimalModel::from_coefficients(&coefficients.decimal);

        let hoodi: Vec<_> = rows
            .iter()
            .filter(|row| row.network == "taiko_hoodi")
            .collect();
        let mainnet: Vec<_> = rows
            .iter()
            .filter(|row| row.network == "taiko_mainnet")
            .collect();

        let hoodi_continuous = diagnostics(&hoodi, |row| decimal.predict(row));
        let hoodi_integer = diagnostics(&hoodi, |row| integer_predict(&coefficients.scaled, row));
        assert_hoodi_diagnostics(
            &hoodi_continuous,
            &artifact.proposal.cohorts.hoodi.continuous,
            17,
            "0.094557",
            "0.279512",
            "0.279512",
            0,
        );
        assert_hoodi_diagnostics(
            &hoodi_integer,
            &artifact.proposal.cohorts.hoodi.scaled_integer,
            12,
            "0.093492",
            "0.264550",
            "0.264550",
            0,
        );

        let mainnet_continuous = diagnostics(&mainnet, |row| decimal.predict(row));
        let mainnet_integer =
            diagnostics(&mainnet, |row| integer_predict(&coefficients.scaled, row));
        assert_mainnet_diagnostics(
            &mainnet_continuous,
            &artifact.proposal.cohorts.mainnet.continuous,
            19,
            "5.87",
            "5.75",
            1,
            "21.94",
        );
        assert_mainnet_diagnostics(
            &mainnet_integer,
            &artifact.proposal.cohorts.mainnet.scaled_integer,
            19,
            "5.8422",
            "5.7234",
            1,
            "21.9679",
        );
    }

    const VALIDATION_FIXTURE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../experiments/risc0-zkgas/models/risc0-zkgas-m2-v1-validation.jsonl"
    ));

    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct ValidationFixtureRow {
        network: String,
        split: String,
        proposal_id: u64,
        block_count: u64,
        total_zkgas: u64,
        actual_mcycles: u64,
    }

    struct DecimalModel {
        intercept: f64,
        total_zkgas: f64,
        block_count: f64,
    }

    impl DecimalModel {
        fn from_coefficients(coefficients: &super::DecimalCoefficients) -> Self {
            Self {
                intercept: parse_audit_decimal(&coefficients.intercept, "intercept"),
                total_zkgas: parse_audit_decimal(&coefficients.total_zkgas, "total_zkgas"),
                block_count: parse_audit_decimal(&coefficients.block_count, "block_count"),
            }
        }

        fn predict(&self, row: &ValidationFixtureRow) -> f64 {
            self.intercept
                + self.total_zkgas * row.total_zkgas as f64
                + self.block_count * row.block_count as f64
        }
    }

    #[derive(Default)]
    struct Diagnostics {
        underquote_count: u32,
        mape_percent: f64,
        max_absolute_error_percent: f64,
        max_underquote_percent: f64,
        over_ten_percent_count: u32,
        max_overquote_percent: f64,
    }

    fn fixture_artifact() -> super::ValidatedModelArtifact {
        let artifact: super::ValidatedModelArtifact =
            serde_json::from_str(super::EMBEDDED_MODEL).expect("committed model artifact JSON");
        super::validate_artifact(&artifact).expect("committed model artifact validation");
        artifact
    }

    fn validation_fixture_rows() -> Vec<ValidationFixtureRow> {
        let mut rows = Vec::new();
        let mut identities = HashSet::new();
        for (line_number, line) in VALIDATION_FIXTURE.lines().enumerate() {
            assert!(!line.trim().is_empty(), "fixture has an empty row");
            let row: ValidationFixtureRow = serde_json::from_str(line)
                .unwrap_or_else(|error| panic!("fixture row {}: {error}", line_number + 1));
            assert!(row.proposal_id > 0, "fixture proposal_id must be positive");
            assert!(row.block_count > 0, "fixture block_count must be positive");
            assert!(row.total_zkgas > 0, "fixture total_zkgas must be positive");
            assert!(
                row.actual_mcycles > 0,
                "fixture actual_mcycles must be positive"
            );
            assert!(
                matches!(
                    (row.network.as_str(), row.split.as_str()),
                    ("taiko_hoodi", "calibration") | ("taiko_mainnet", "evaluation")
                ),
                "fixture has an unsupported cohort"
            );
            assert!(
                identities.insert((row.network.clone(), row.proposal_id)),
                "fixture has duplicate network/proposal_id"
            );
            rows.push(row);
        }
        rows
    }

    fn parse_audit_decimal(value: &str, label: &str) -> f64 {
        let parsed = f64::from_str(value)
            .unwrap_or_else(|error| panic!("{label} audit decimal is malformed: {error}"));
        assert!(
            parsed.is_finite() && parsed != 0.0,
            "{label} audit decimal must be finite and non-zero"
        );
        parsed
    }

    fn integer_predict(
        coefficients: &super::ScaledCoefficients,
        row: &ValidationFixtureRow,
    ) -> f64 {
        let numerator = u128::from(coefficients.intercept)
            + u128::from(coefficients.total_zkgas) * u128::from(row.total_zkgas)
            + u128::from(coefficients.block_count) * u128::from(row.block_count);
        let scale = u128::from(coefficients.scale);
        assert!(scale > 0, "scaled coefficients require a non-zero scale");
        (numerator / scale + u128::from(!numerator.is_multiple_of(scale))) as f64
    }

    fn diagnostics(
        rows: &[&ValidationFixtureRow],
        prediction: impl Fn(&ValidationFixtureRow) -> f64,
    ) -> Diagnostics {
        assert!(!rows.is_empty(), "diagnostics require fixture rows");
        let mut diagnostics = Diagnostics::default();
        for row in rows {
            let percent_error =
                (prediction(row) - row.actual_mcycles as f64) * 100.0 / row.actual_mcycles as f64;
            diagnostics.mape_percent += percent_error.abs();
            diagnostics.max_absolute_error_percent = diagnostics
                .max_absolute_error_percent
                .max(percent_error.abs());
            if percent_error < 0.0 {
                diagnostics.underquote_count += 1;
                diagnostics.max_underquote_percent =
                    diagnostics.max_underquote_percent.max(-percent_error);
            }
            if percent_error > 10.0 {
                diagnostics.over_ten_percent_count += 1;
                diagnostics.max_overquote_percent =
                    diagnostics.max_overquote_percent.max(percent_error);
            }
        }
        diagnostics.mape_percent /= rows.len() as f64;
        diagnostics
    }

    fn assert_hoodi_diagnostics(
        actual: &Diagnostics,
        artifact: &super::HoodiDiagnostics,
        underquote_count: u32,
        mape_percent: &str,
        max_absolute_error_percent: &str,
        max_underquote_percent: &str,
        over_ten_percent_count: u32,
    ) {
        assert_eq!(artifact.underquote_count, underquote_count);
        assert_eq!(artifact.mape_percent, mape_percent);
        assert_eq!(
            artifact.max_absolute_error_percent,
            max_absolute_error_percent
        );
        assert_eq!(artifact.max_underquote_percent, max_underquote_percent);
        assert_eq!(artifact.over_ten_percent_count, over_ten_percent_count);
        assert_eq!(actual.underquote_count, artifact.underquote_count);
        assert_percent_eq(actual.mape_percent, &artifact.mape_percent);
        assert_percent_eq(
            actual.max_absolute_error_percent,
            &artifact.max_absolute_error_percent,
        );
        assert_percent_eq(
            actual.max_underquote_percent,
            &artifact.max_underquote_percent,
        );
        assert_eq!(
            actual.over_ten_percent_count,
            artifact.over_ten_percent_count
        );
    }

    fn assert_mainnet_diagnostics(
        actual: &Diagnostics,
        artifact: &super::MainnetDiagnostics,
        underquote_count: u32,
        mape_percent: &str,
        max_underquote_percent: &str,
        overquote_over_ten_percent_count: u32,
        max_overquote_percent: &str,
    ) {
        assert_eq!(artifact.underquote_count, underquote_count);
        assert_eq!(artifact.mape_percent, mape_percent);
        assert_eq!(artifact.max_underquote_percent, max_underquote_percent);
        assert_eq!(
            artifact.overquote_over_ten_percent_count,
            overquote_over_ten_percent_count
        );
        assert_eq!(artifact.max_overquote_percent, max_overquote_percent);
        assert_eq!(actual.underquote_count, artifact.underquote_count);
        assert_percent_eq(actual.mape_percent, &artifact.mape_percent);
        assert_percent_eq(
            actual.max_underquote_percent,
            &artifact.max_underquote_percent,
        );
        assert_eq!(
            actual.over_ten_percent_count,
            artifact.overquote_over_ten_percent_count
        );
        assert_percent_eq(
            actual.max_overquote_percent,
            &artifact.max_overquote_percent,
        );
    }

    fn assert_percent_eq(actual: f64, expected: &str) {
        let decimals = expected
            .split_once('.')
            .map_or(0, |(_, fraction)| fraction.len());
        let expected = parse_audit_decimal(expected, "diagnostic");
        let tolerance =
            0.5 * 10_f64.powi(-i32::try_from(decimals).expect("decimal precision")) + 1e-9;
        assert!(
            (actual - expected).abs() <= tolerance,
            "expected {expected} +/- {tolerance}, got {actual}"
        );
    }
}
