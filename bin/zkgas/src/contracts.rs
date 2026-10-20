//! Independently versioned, strict composition contracts. Pricing remains in zkgas-trace.
use std::collections::BTreeMap;

use raiko2_zkgas_trace::ProposalWorkloadQuote;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};
use serde_json::Value;

pub const MAX_OPAQUE_BYTES: usize = 65_536;
pub const MAX_OPAQUE_DEPTH: usize = 16;
pub const MAX_OPAQUE_ENTRIES: usize = 1_024;
pub const MAX_INPUT_ROWS: usize = 10_000;
pub const MAX_OBSERVATION_ROWS: usize = 10_000;

fn version_one<'de, D: Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    let version = u32::deserialize(d)?;
    if version != 1 {
        return Err(D::Error::custom("unsupported schema_version; expected 1"));
    }
    Ok(version)
}

/// Opaque producer provenance, bounded across the entire nested object.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct OpaqueObject(BTreeMap<String, Value>);

impl OpaqueObject {
    pub fn new(values: BTreeMap<String, Value>) -> Result<Self, String> {
        let bytes = serde_json::to_vec(&values).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_OPAQUE_BYTES {
            return Err("opaque object byte limit exceeded".into());
        }
        fn visit(value: &Value, depth: usize, entries: &mut usize) -> Result<(), String> {
            if depth > MAX_OPAQUE_DEPTH {
                return Err("opaque object depth limit exceeded".into());
            }
            match value {
                Value::Object(map) => {
                    *entries += map.len();
                    for child in map.values() {
                        visit(child, depth + 1, entries)?;
                    }
                }
                Value::Array(array) => {
                    *entries += array.len();
                    for child in array {
                        visit(child, depth + 1, entries)?;
                    }
                }
                _ => {}
            }
            if *entries > MAX_OPAQUE_ENTRIES {
                return Err("opaque object entry limit exceeded".into());
            }
            Ok(())
        }
        visit(
            &serde_json::to_value(&values).map_err(|e| e.to_string())?,
            0,
            &mut 0,
        )?;
        Ok(Self(values))
    }

    pub fn as_map(&self) -> &BTreeMap<String, Value> {
        &self.0
    }
}

impl<'de> Deserialize<'de> for OpaqueObject {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Self::new(BTreeMap::deserialize(d)?).map_err(D::Error::custom)
    }
}

macro_rules! wire {
    ($(#[$meta:meta])* pub struct $name:ident { $($(#[$field:meta])* pub $key:ident : $ty:ty),* $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub struct $name { $($(#[$field])* pub $key: $ty),* }
    };
}

wire! { pub struct ExecutableIdentity {
    pub sha256: String,
    pub package_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub source_revision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub dirty: Option<bool>,
} }
wire! { #[derive(PartialOrd, Ord)] pub struct QuoteKey {
    pub corpus_id: String,
    pub sample_id: String,
    pub guest_input_sha256: String,
} }
wire! { #[derive(PartialOrd, Ord)] pub struct ObservationKey {
    pub observation_set_id: String,
    pub observation_id: String,
    pub attempt_id: String,
} }
wire! { pub struct InputFile {
    pub path: String,
    pub sha256: String,
    pub byte_length: u64,
} }
wire! { pub struct InputSampleV1 {
    pub sample_id: String,
    pub guest_input: InputFile,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub metadata: Option<OpaqueObject>,
} }

fn bounded_rows<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
    limit: usize,
    limit_error: &'static str,
) -> Result<Vec<T>, D::Error> {
    struct Rows<T> {
        limit: usize,
        limit_error: &'static str,
        row: std::marker::PhantomData<T>,
    }
    // SeqAccess determines whether another element exists; the seed rejects it before decoding
    // or allocating that row. An invalid later tail cannot mask a reached count limit.
    struct RejectExtraRow(&'static str);
    impl<'de> serde::de::DeserializeSeed<'de> for RejectExtraRow {
        type Value = ();
        fn deserialize<D: Deserializer<'de>>(self, _d: D) -> Result<(), D::Error> {
            Err(D::Error::custom(self.0))
        }
    }
    impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for Rows<T> {
        type Value = Vec<T>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("a bounded row list")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut seq: A,
        ) -> Result<Self::Value, A::Error> {
            let mut rows = Vec::new();
            while rows.len() < self.limit {
                let Some(row) = seq.next_element()? else {
                    return Ok(rows);
                };
                rows.push(row);
            }
            seq.next_element_seed(RejectExtraRow(self.limit_error))?;
            Ok(rows)
        }
    }
    d.deserialize_seq(Rows {
        limit,
        limit_error,
        row: std::marker::PhantomData,
    })
}

fn bounded_samples<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<InputSampleV1>, D::Error> {
    bounded_rows(d, MAX_INPUT_ROWS, "input manifest row limit exceeded")
}

fn bounded_observations<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Vec<ObservationRowV1>, D::Error> {
    bounded_rows(
        d,
        MAX_OBSERVATION_ROWS,
        "observation manifest row limit exceeded",
    )
}
wire! { pub struct InputManifestV1 {
    #[serde(deserialize_with = "version_one")] pub schema_version: u32,
    pub corpus_id: String,
    #[serde(deserialize_with = "bounded_samples")] pub samples: Vec<InputSampleV1>,
} }

wire! { /// Digest algorithms are explicit and must be compared together with their values.
#[derive(PartialOrd, Ord)]
pub struct ProgramIdentity {
    pub elf_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub serialized_vk_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub sp1_vkey: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub risc0_image_id: Option<String>,
} }
wire! { pub struct Workload {
    #[serde(with = "decimal_u128")] pub value: u128,
    pub unit: String,
} }
wire! { pub struct FailureV1 {
    pub code: String,
    pub message: String,
} }
wire! { pub struct CompleteObservationV1 {
    pub observation_id: String,
    pub attempt_id: String,
    pub quote_key: QuoteKey,
    pub backend: String,
    pub workload: Workload,
    pub measurement_stage: String,
    pub program: ProgramIdentity,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub backend_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub software_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub replicate_group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub source: Option<OpaqueObject>,
} }
wire! { pub struct FailedObservationV1 {
    pub observation_id: String,
    pub attempt_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub quote_key: Option<QuoteKey>,
    pub failure: FailureV1,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub source: Option<OpaqueObject>,
} }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum ObservationRowV1 {
    Complete(Box<CompleteObservationV1>),
    Failed(Box<FailedObservationV1>),
}
wire! { pub struct ObservationManifestV1 {
    #[serde(deserialize_with = "version_one")] pub schema_version: u32,
    pub observation_set_id: String,
    #[serde(deserialize_with = "bounded_observations")] pub observations: Vec<ObservationRowV1>,
} }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum InputSourceV1 {
    Manifest {
        manifest_sha256: String,
        row_ordinal: u64,
    },
    Single {
        descriptor_sha256: String,
    },
}
wire! { pub struct ChainIdentityV1 {
    pub name: String,
    pub chain_id: u64,
} }
wire! { #[derive(PartialOrd, Ord)] pub struct ForkSegmentV1 {
    pub fork: String,
    pub first_block: u64,
    pub last_block: u64,
} }
wire! { pub struct ProposalIdentityV1 {
    pub proposal_id: u64,
    pub chain: ChainIdentityV1,
    pub proof_carry_chain_id: u64,
    pub proof_carry_proposal_id: u64,
    pub witness_chain_spec_sha256: Vec<String>,
    pub first_block: u64,
    pub last_block: u64,
    pub block_count: u64,
    pub public_output: String,
    pub fork_segments: Vec<ForkSegmentV1>,
} }
wire! { pub struct CompleteQuoteV1 {
    #[serde(deserialize_with = "version_one")] pub schema_version: u32,
    pub key: QuoteKey,
    pub executable: ExecutableIdentity,
    pub input_source: InputSourceV1,
    pub guest_input_byte_length: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub metadata: Option<OpaqueObject>,
    pub proposal: ProposalIdentityV1,
    pub prediction_unit: String,
    pub prediction_surface: String,
    pub calibration_profile: String,
    #[serde(with = "quote_wire")] pub quote: ProposalWorkloadQuote,
} }
wire! { pub struct FailedQuoteV1 {
    #[serde(deserialize_with = "version_one")] pub schema_version: u32,
    pub corpus_id: String,
    pub sample_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub verified_key: Option<QuoteKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub observed_guest_input_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub observed_guest_input_byte_length: Option<u64>,
    pub executable: ExecutableIdentity,
    pub input_source: InputSourceV1,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub metadata: Option<OpaqueObject>,
    pub failure: FailureV1,
} }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum QuoteRowV1 {
    Complete(Box<CompleteQuoteV1>),
    Failed(Box<FailedQuoteV1>),
}

wire! { pub struct NormalizationV1 {
    pub profile_id: String,
    pub source_unit: String,
    pub target_unit: String,
    pub source_program: ProgramIdentity,
    pub target_program: ProgramIdentity,
    pub numerator: String,
    pub denominator: String,
    pub artifact_path: String,
    pub artifact_sha256: String,
    pub artifact_identity: String,
    pub diagnostic_only: bool,
} }
wire! { pub struct CompatibilityProfileV1 {
    pub profile_id: String,
    pub model_id: String,
    pub backend: String,
    pub prediction_unit: String,
    pub observation_unit: String,
    pub measurement_stage: String,
    pub program: ProgramIdentity,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub backend_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub runtime_spec_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub normalization: Option<NormalizationV1>,
} }
wire! { pub struct CompatibilityProfilesV1 {
    #[serde(deserialize_with = "version_one")] pub schema_version: u32,
    pub profiles: Vec<CompatibilityProfileV1>,
} }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Complete,
    Partial,
}
wire! { pub struct RowDigestV1 {
    pub comparison_row_id: String,
    pub row_sha256: String,
} }
wire! { pub struct ComparisonManifestV1 {
    #[serde(deserialize_with = "version_one")] pub schema_version: u32,
    pub run_id: String,
    pub executable: ExecutableIdentity,
    pub quote_file_sha256: Vec<String>,
    pub observation_file_sha256: Vec<String>,
    pub profiles_sha256: String,
    pub arithmetic_policy: String,
    pub model_ids: Vec<String>,
    pub profile_ids: Vec<String>,
    pub rows: Vec<RowDigestV1>,
    pub row_sequence_sha256: String,
    pub status: RunStatus,
    pub counts: BTreeMap<ComparisonKind, u64>,
} }
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonKind {
    Joined,
    IncompatibleProfile,
    AmbiguousProfile,
    UnmatchedQuote,
    UnmatchedObservation,
    InvalidQuote,
    InvalidObservation,
}
wire! { pub struct RatioV1 {
    pub numerator: String,
    pub denominator: String,
    pub display: String,
} }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActualClassification {
    Positive,
    ZeroActual,
    InfiniteUnderpricing,
}
wire! { pub struct DirectMetricsV1 {
    pub classification: ActualClassification,
    pub underquoted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub prediction_over_actual: Option<RatioV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub actual_over_prediction: Option<RatioV1>,
} }
wire! { pub struct NormalizedDiagnosticV1 {
    pub normalization_profile_id: String,
    pub normalization_sha256: String,
    pub source_unit: String,
    pub target_unit: String,
    pub converted_prediction: String,
    pub diagnostic_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub prediction_over_actual: Option<RatioV1>,
} }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ComparisonResultV1 {
    Joined {
        quote: Box<CompleteQuoteV1>,
        observation: CompleteObservationV1,
        observation_key: ObservationKey,
        profile_id: String,
        metrics: DirectMetricsV1,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        normalization: Option<NormalizedDiagnosticV1>,
    },
    IncompatibleProfile {
        quote: Box<CompleteQuoteV1>,
        observation: CompleteObservationV1,
        observation_key: ObservationKey,
        profiles_sha256: String,
        reason: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cross_program_ratio: Option<RatioV1>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        normalization: Option<NormalizedDiagnosticV1>,
    },
    AmbiguousProfile {
        quote: Box<CompleteQuoteV1>,
        observation: CompleteObservationV1,
        observation_key: ObservationKey,
        matching_profile_ids: Vec<String>,
    },
    UnmatchedQuote {
        quote: Box<CompleteQuoteV1>,
        quote_row_sha256: String,
    },
    UnmatchedObservation {
        observation: CompleteObservationV1,
        observation_key: ObservationKey,
        observation_row_sha256: String,
    },
    InvalidQuote {
        quote: FailedQuoteV1,
        quote_row_sha256: String,
    },
    InvalidObservation {
        observation_set_id: String,
        observation: FailedObservationV1,
        observation_row_sha256: String,
    },
}
wire! { pub struct ComparisonRowV1 {
    #[serde(deserialize_with = "version_one")] pub schema_version: u32,
    pub comparison_row_id: String,
    pub result: ComparisonResultV1,
} }
wire! { #[derive(Default)] pub struct ReportFiltersV1 {
    #[serde(default, skip_serializing_if = "Vec::is_empty")] pub chain_ids: Vec<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")] pub backends: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")] pub program_elf_sha256: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")] pub model_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")] pub profile_ids: Vec<String>,
} }
wire! { pub struct WorkloadBucketV1 {
    pub name: String,
    #[serde(with = "decimal_u128")] pub minimum: u128,
    #[serde(default, with = "optional_decimal_u128", skip_serializing_if = "Option::is_none")] pub maximum_exclusive: Option<u128>,
} }
wire! { pub struct ReportConfigV1 {
    #[serde(deserialize_with = "version_one")] pub schema_version: u32,
    pub filters: ReportFiltersV1,
    pub quantiles: Vec<String>,
    pub buckets: Vec<WorkloadBucketV1>,
} }
wire! { #[derive(PartialOrd, Ord)] pub struct StratumIdentityV1 {
    pub chain_id: u64,
    pub fork_segments: Vec<ForkSegmentV1>,
    pub backend: String,
    pub workload_unit: String,
    pub measurement_stage: String,
    pub program: ProgramIdentity,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub backend_version: Option<String>,
    pub model_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub profile_id: Option<String>,
    pub compatible: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub normalization_sha256: Option<String>,
} }
wire! { pub struct MetricDiagnosticV1 {
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub unavailable_reason: Option<String>,
} }
wire! { pub struct BucketSummaryV1 {
    pub name: String,
    pub count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub amplification: Option<RatioV1>,
} }
wire! { pub struct StratumReportV1 {
    pub identity: StratumIdentityV1,
    pub row_count: u64,
    pub eligible_count: u64,
    pub zero_actual_count: u64,
    pub infinite_underpricing_count: u64,
    pub prediction_sum: String,
    pub actual_sum: String,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub count_weighted_prediction_over_actual: Option<RatioV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub workload_weighted_prediction_over_actual: Option<RatioV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub underquote_rate: Option<RatioV1>,
    pub quantile_method: String,
    pub quantiles: BTreeMap<String, RatioV1>,
    pub diagnostics: BTreeMap<String, MetricDiagnosticV1>,
    pub buckets: Vec<BucketSummaryV1>,
} }
wire! { pub struct ExclusionV1 {
    pub comparison_row_id: String,
    pub reason: String,
} }
wire! { pub struct ReportV1 {
    #[serde(deserialize_with = "version_one")] pub schema_version: u32,
    pub report_id: String,
    pub comparison_manifest_sha256: String,
    pub row_sequence_sha256: String,
    pub config_sha256: String,
    pub executable: ExecutableIdentity,
    pub arithmetic_policy: String,
    pub status: RunStatus,
    pub whole_run_counts: BTreeMap<ComparisonKind, u64>,
    pub selected_counts: BTreeMap<ComparisonKind, u64>,
    pub strata: Vec<StratumReportV1>,
    pub excluded: Vec<ExclusionV1>,
} }

pub mod decimal_u128 {
    use super::*;
    pub fn serialize<S: Serializer>(value: &u128, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&value.to_string())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u128, D::Error> {
        let text = String::deserialize(d)?;
        if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
            return Err(D::Error::custom("expected unsigned decimal string"));
        }
        text.parse().map_err(D::Error::custom)
    }
}
mod optional_decimal_u128 {
    use super::*;
    pub fn serialize<S: Serializer>(value: &Option<u128>, s: S) -> Result<S::Ok, S::Error> {
        value.map(|v| v.to_string()).serialize(s)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u128>, D::Error> {
        let value = Option::<String>::deserialize(d)?;
        value
            .map(|text| {
                if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(D::Error::custom("expected unsigned decimal string"));
                }
                text.parse().map_err(D::Error::custom)
            })
            .transpose()
    }
}

/// Preserve the canonical quote type without losing its i128/u128 values at the JSON boundary.
mod quote_wire {
    use super::*;
    fn wide_integer(key: &str) -> bool {
        key.ends_with("_scaled") || key.ends_with("_pgu") || key == "unzen_zkgas_sum"
    }
    fn transform(value: &mut Value, encoding: bool) -> Result<(), String> {
        match value {
            Value::Object(object) => {
                for (key, value) in object {
                    if wide_integer(key) {
                        if encoding {
                            let number = value.as_number().ok_or("expected quote integer")?;
                            *value = Value::String(number.to_string());
                        } else {
                            let text = value
                                .as_str()
                                .ok_or("quote wide integers must be decimal strings")?;
                            let digits = text.strip_prefix('-').unwrap_or(text);
                            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                                return Err("invalid quote decimal string".into());
                            }
                            *value =
                                Value::Number(text.parse().map_err(|_| "invalid quote integer")?);
                        }
                    } else {
                        transform(value, encoding)?;
                    }
                }
            }
            Value::Array(array) => {
                for value in array {
                    transform(value, encoding)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    pub fn serialize<S: Serializer>(
        quote: &ProposalWorkloadQuote,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        let mut value = serde_json::to_value(quote).map_err(serde::ser::Error::custom)?;
        transform(&mut value, true).map_err(serde::ser::Error::custom)?;
        value.serialize(s)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<ProposalWorkloadQuote, D::Error> {
        let mut value = Value::deserialize(d)?;
        transform(&mut value, false).map_err(D::Error::custom)?;
        let quote: ProposalWorkloadQuote =
            serde_json::from_value(value).map_err(D::Error::custom)?;
        if quote.schema_version != 1 {
            return Err(D::Error::custom(
                "unsupported embedded quote schema_version",
            ));
        }
        Ok(quote)
    }
}
