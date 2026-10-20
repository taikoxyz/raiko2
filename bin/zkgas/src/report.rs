//! Integrity-checked, bounded, deterministic reports over immutable comparison runs.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use num_bigint::{BigInt, BigUint};
use num_rational::BigRational;
use num_traits::{One, Signed, ToPrimitive, Zero};
use rustix::fs::{CWD, Mode, OFlags, ResolveFlags, openat2};
use serde::de::{DeserializeSeed, Error as _, MapAccess, SeqAccess, Visitor};
use serde_json::Value;

use crate::{
    arithmetic::ARITHMETIC_POLICY,
    compare::{
        MAX_COMPARE_ROWS, comparison_kind, comparison_row_id, comparison_run_id,
        row_sequence_sha256,
    },
    contracts::*,
    identity::{canonical_json, content_id, sha256_hex},
    output::{CommitOutcome, commit_file_bounded},
};

pub const MAX_REPORT_MANIFEST_BYTES: u64 = 4 * 1024 * 1024;
pub const MAX_REPORT_ROW_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_REPORT_TOTAL_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_REPORT_CONFIG_BYTES: u64 = 64 * 1024;
pub const MAX_REPORT_CONFIG_ITEMS: usize = 128;
pub const MAX_REPORT_OUTPUT_BYTES: u64 = 32 * 1024 * 1024;
const GROUPING_POLICY: &str = "full-stratum-v1;chain;ordered-fork-topology-relative-transition-offsets;backend;unit;stage;full-program;backend-version;model;resolved-profile;compatibility;normalization-wrapper";
const METRIC_POLICY: &str = "report-v1;per-row-ratios-and-underquote-positive-actual-only;workload-ratio-full-sums;nearest-rank-max(1,ceil(q*n));raw-pearson;spearman-average-ties;kendall-tau-b;sqrt-rational-half-even-6;prediction-buckets;amplification=max(actual)/min(positive-actual);zero-or-less-than-two-positive-unavailable";

#[derive(Clone, Debug, clap::Parser)]
#[command(about = "Report verified comparison files without data fetching or guest execution")]
#[command(
    after_help = "Hard ceilings: 10,000 rows; 4 MiB manifest; 8 MiB per row; 256 MiB total; 64 KiB config; 128 items per config collection; 32 MiB report. Entire immutable tree is verified before filtering. Config accepts strict JSON or TOML."
)]
pub struct ReportArgs {
    #[arg(long)]
    pub comparisons: PathBuf,
    #[arg(long)]
    pub config: PathBuf,
    #[arg(long)]
    pub output: PathBuf,
}

/// Fields are private: a report cannot be built from unverified or subsequently mutated rows.
#[derive(Debug)]
pub struct VerifiedComparisonRun {
    manifest: ComparisonManifestV1,
    manifest_sha256: String,
    rows: Vec<ComparisonRowV1>,
}

#[derive(Debug)]
pub struct ReportRun {
    pub report: ReportV1,
    pub commit: CommitOutcome,
}
impl ReportRun {
    pub fn exit_code(&self) -> i32 {
        match self.report.status {
            RunStatus::Complete => 0,
            RunStatus::Partial => 2,
        }
    }
}

fn open_regular(path: &Path) -> Result<File> {
    let file = File::from(openat2(
        CWD,
        path,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )?);
    ensure!(file.metadata()?.is_file(), "input is not a regular file");
    Ok(file)
}
fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let file = open_regular(path)?;
    let length = file.metadata()?.len();
    ensure!(length <= limit, "file byte limit exceeded");
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 == length,
        "input length changed during read"
    );
    ensure!(bytes.len() as u64 <= limit, "file byte limit exceeded");
    Ok(bytes)
}

/// A lexical allocation gate, run BEFORE typed JSON deserialization. It bounds every array,
/// string, token and nesting stack and stops at the first excess comma, without decoding its tail.
/// The strict serde parser subsequently checks the complete JSON grammar and contract.
fn json_budget(bytes: &[u8], array_limit: usize, string_limit: usize) -> Result<()> {
    struct Frame {
        array: bool,
        count: usize,
        expecting: bool,
    }
    let mut stack: Vec<Frame> = Vec::new();
    let mut i = 0;
    let mut tokens = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        tokens += 1;
        ensure!(tokens <= 250_000, "JSON token limit exceeded");
        if b == b',' {
            if let Some(f) = stack.last_mut() {
                ensure!(
                    !f.array || f.count < array_limit,
                    "JSON array row/item limit exceeded"
                );
                f.expecting = true;
            }
            i += 1;
            continue;
        }
        if b == b':' {
            i += 1;
            continue;
        }
        if b == b']' || b == b'}' {
            stack.pop();
            i += 1;
            continue;
        }
        if let Some(f) = stack.last_mut()
            && f.array
            && f.expecting
        {
            f.count += 1;
            f.expecting = false;
            ensure!(f.count <= array_limit, "JSON array row/item limit exceeded");
        }
        if b == b'[' || b == b'{' {
            ensure!(stack.len() < 64, "JSON nesting limit exceeded");
            stack.push(Frame {
                array: b == b'[',
                count: 0,
                expecting: true,
            });
            i += 1;
        } else if b == b'"' {
            i += 1;
            let start = i;
            while i < bytes.len() {
                ensure!(
                    i - start <= string_limit,
                    "JSON string length limit exceeded"
                );
                if bytes[i] == b'"' {
                    break;
                }
                if bytes[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            ensure!(
                i - start <= string_limit,
                "JSON string length limit exceeded"
            );
            i += 1;
        } else {
            let start = i;
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() && !b",]}:".contains(&bytes[i])
            {
                i += 1;
                ensure!(i - start <= 128, "JSON numeric/token length limit exceeded");
            }
            ensure!(i > start, "invalid JSON token");
        }
    }
    Ok(())
}

fn is_sha256(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub fn verify_comparison_run(path: &Path) -> Result<VerifiedComparisonRun> {
    let meta = fs::symlink_metadata(path)?;
    ensure!(
        meta.is_dir() && !meta.file_type().is_symlink(),
        "comparison run must be a real directory"
    );
    let mut root_names = BTreeSet::new();
    for entry in fs::read_dir(path)? {
        ensure!(root_names.len() < 2, "unexpected comparison tree entry");
        root_names.insert(entry?.file_name());
    }
    ensure!(
        root_names == ["manifest.json".into(), "rows".into()].into(),
        "unexpected comparison tree entries"
    );
    let raw = read_bounded(&path.join("manifest.json"), MAX_REPORT_MANIFEST_BYTES)?;
    json_budget(&raw, MAX_COMPARE_ROWS, 65_536)?;
    let manifest: ComparisonManifestV1 =
        serde_json::from_slice(&raw).context("strict comparison manifest")?;
    ensure!(
        manifest.rows.len() <= MAX_COMPARE_ROWS,
        "comparison row limit exceeded"
    );
    ensure!(
        comparison_run_id(&manifest)? == manifest.run_id,
        "comparison manifest run ID mismatch"
    );
    ensure!(
        path.file_name().and_then(|s| s.to_str()) == Some(manifest.run_id.as_str()),
        "comparison directory run ID mismatch"
    );
    ensure!(
        manifest.arithmetic_policy == ARITHMETIC_POLICY,
        "unsupported comparison arithmetic policy"
    );
    ensure!(
        row_sequence_sha256(&manifest.rows)? == manifest.row_sequence_sha256,
        "comparison row sequence mismatch"
    );
    let mut declared = BTreeSet::new();
    for digest in &manifest.rows {
        ensure!(
            is_sha256(&digest.comparison_row_id) && is_sha256(&digest.row_sha256),
            "invalid row digest/ID"
        );
        ensure!(
            declared.insert(format!("{}.json", digest.comparison_row_id)),
            "duplicate comparison row ID"
        );
    }
    let rows_dir = path.join("rows");
    let meta = fs::symlink_metadata(&rows_dir)?;
    ensure!(
        meta.is_dir() && !meta.file_type().is_symlink(),
        "rows must be a real directory"
    );
    let mut actual = BTreeSet::new();
    let mut total = raw.len() as u64;
    for entry in fs::read_dir(&rows_dir)? {
        ensure!(
            actual.len() < MAX_COMPARE_ROWS,
            "comparison file count limit exceeded"
        );
        let entry = entry?;
        let meta = fs::symlink_metadata(entry.path())?;
        ensure!(
            meta.is_file() && !meta.file_type().is_symlink(),
            "row must be a regular non-symlink file"
        );
        ensure!(
            meta.len() <= MAX_REPORT_ROW_BYTES,
            "comparison row byte limit exceeded"
        );
        total = total
            .checked_add(meta.len())
            .context("comparison byte count overflow")?;
        ensure!(
            total <= MAX_REPORT_TOTAL_BYTES,
            "comparison total byte limit exceeded"
        );
        actual.insert(
            entry
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("non-UTF8 row filename"))?,
        );
    }
    ensure!(
        actual == declared,
        "comparison row filename set differs from manifest"
    );
    let mut rows = Vec::new();
    let mut counts = BTreeMap::new();
    let mut loaded = raw.len() as u64;
    for digest in &manifest.rows {
        let raw = read_bounded(
            &rows_dir.join(format!("{}.json", digest.comparison_row_id)),
            MAX_REPORT_ROW_BYTES.min(MAX_REPORT_TOTAL_BYTES - loaded),
        )?;
        loaded += raw.len() as u64;
        ensure!(
            sha256_hex(&raw) == digest.row_sha256,
            "comparison row content hash mismatch"
        );
        json_budget(&raw, MAX_COMPARE_ROWS, 65_536)?;
        let row: ComparisonRowV1 = serde_json::from_slice(&raw).context("strict comparison row")?;
        ensure!(
            row.comparison_row_id == digest.comparison_row_id
                && comparison_row_id(&row.result)? == row.comparison_row_id,
            "comparison tagged row ID mismatch"
        );
        *counts.entry(comparison_kind(&row.result)).or_insert(0u64) += 1;
        rows.push(row);
    }
    ensure!(counts == manifest.counts, "comparison counts mismatch");
    let status = if counts.keys().all(|k| {
        matches!(
            k,
            ComparisonKind::Joined | ComparisonKind::IncompatibleProfile
        )
    }) {
        RunStatus::Complete
    } else {
        RunStatus::Partial
    };
    ensure!(status == manifest.status, "comparison status mismatch");
    Ok(VerifiedComparisonRun {
        manifest,
        manifest_sha256: sha256_hex(&raw),
        rows,
    })
}

/// Bounded strict config visitor. JSON has an additional lexical allocation gate; TOML's
/// authoritative parser first constructs its document, bounded by the 64 KiB raw input ceiling.
struct ConfigBudget<'a> {
    total: &'a mut usize,
    depth: usize,
}
impl<'de> DeserializeSeed<'de> for ConfigBudget<'_> {
    type Value = Value;
    fn deserialize<D: serde::Deserializer<'de>>(
        self,
        d: D,
    ) -> std::result::Result<Value, D::Error> {
        ensure_config(self.depth <= 16 && *self.total < 4096).map_err(D::Error::custom)?;
        *self.total += 1;
        d.deserialize_any(self)
    }
}
fn ensure_config(ok: bool) -> std::result::Result<(), &'static str> {
    if ok {
        Ok(())
    } else {
        Err("report config depth/node limit exceeded")
    }
}
impl<'de> Visitor<'de> for ConfigBudget<'_> {
    type Value = Value;
    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("bounded config value")
    }
    fn visit_str<E: serde::de::Error>(self, v: &str) -> std::result::Result<Value, E> {
        if v.len() > 1024 {
            return Err(E::custom("config string length limit exceeded"));
        }
        Ok(Value::String(v.into()))
    }
    fn visit_string<E: serde::de::Error>(self, v: String) -> std::result::Result<Value, E> {
        self.visit_str(&v)
    }
    fn visit_u64<E: serde::de::Error>(self, v: u64) -> std::result::Result<Value, E> {
        Ok(v.into())
    }
    fn visit_i64<E: serde::de::Error>(self, v: i64) -> std::result::Result<Value, E> {
        Ok(v.into())
    }
    fn visit_bool<E: serde::de::Error>(self, v: bool) -> std::result::Result<Value, E> {
        Ok(v.into())
    }
    fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<Value, A::Error> {
        struct Reject;
        impl<'de> DeserializeSeed<'de> for Reject {
            type Value = ();
            fn deserialize<D: serde::Deserializer<'de>>(
                self,
                _: D,
            ) -> std::result::Result<(), D::Error> {
                Err(D::Error::custom("report config item limit exceeded"))
            }
        }
        let mut values = Vec::new();
        while values.len() < MAX_REPORT_CONFIG_ITEMS {
            let Some(value) = seq.next_element_seed(ConfigBudget {
                total: self.total,
                depth: self.depth + 1,
            })?
            else {
                return Ok(Value::Array(values));
            };
            values.push(value);
        }
        seq.next_element_seed(Reject)?;
        Ok(Value::Array(values))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<Value, A::Error> {
        let mut values = serde_json::Map::new();
        while let Some(key) = map.next_key::<String>()? {
            if values.len() >= MAX_REPORT_CONFIG_ITEMS
                || key.len() > 1024
                || values.contains_key(&key)
            {
                return Err(A::Error::custom(
                    "config key count/length/duplicate limit exceeded",
                ));
            }
            let value = map.next_value_seed(ConfigBudget {
                total: self.total,
                depth: self.depth + 1,
            })?;
            values.insert(key, value);
        }
        Ok(Value::Object(values))
    }
}
fn parse_config(raw: &[u8]) -> Result<ReportConfigV1> {
    let text = std::str::from_utf8(raw)?;
    let value = if text.trim_start().starts_with('{') {
        json_budget(raw, MAX_REPORT_CONFIG_ITEMS, 1024)?;
        let mut decoder = serde_json::Deserializer::from_slice(raw);
        let value = ConfigBudget {
            total: &mut 0,
            depth: 0,
        }
        .deserialize(&mut decoder)?;
        decoder.end()?;
        value
    } else {
        // The TOML adapter parses its whole document before invoking this visitor. The raw
        // read ceiling bounds parser input; these checks bound the resulting semantic config.
        ConfigBudget {
            total: &mut 0,
            depth: 0,
        }
        .deserialize(toml::Deserializer::new(text))?
    };
    if let Some(buckets) = value.get("buckets").and_then(Value::as_array) {
        for bucket in buckets {
            for key in ["minimum", "maximum_exclusive"] {
                if let Some(text) = bucket.get(key).and_then(Value::as_str) {
                    ensure!(text.len() <= 39, "bucket decimal length limit exceeded");
                }
            }
        }
    }
    Ok(serde_json::from_value(value)?)
}

fn quantile(text: &str) -> Result<BigRational> {
    ensure!(
        !text.is_empty() && text.len() <= 32,
        "quantile decimal length limit exceeded"
    );
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    ensure!(
        matches!(whole, "0" | "1")
            && fraction.len() <= 18
            && fraction.bytes().all(|b| b.is_ascii_digit())
            && (!text.contains('.') || !fraction.is_empty()),
        "unsupported quantile decimal"
    );
    let denominator = BigInt::from(10u32).pow(fraction.len() as u32);
    let numerator: BigInt = format!("{whole}{fraction}").parse()?;
    ensure!(numerator <= denominator, "quantile must be in [0,1]");
    Ok(BigRational::new(numerator, denominator))
}

fn validate_config(config: &ReportConfigV1) -> Result<()> {
    ensure!(
        config.schema_version == 1,
        "unsupported report config version"
    );
    ensure!(
        config.quantiles.len() <= MAX_REPORT_CONFIG_ITEMS
            && config.buckets.len() <= MAX_REPORT_CONFIG_ITEMS,
        "report config item limit exceeded"
    );
    let f = &config.filters;
    ensure!(
        f.chain_ids.len() <= MAX_REPORT_CONFIG_ITEMS,
        "filter count limit exceeded"
    );
    ensure!(
        f.chain_ids.iter().collect::<BTreeSet<_>>().len() == f.chain_ids.len(),
        "duplicate chain filter"
    );
    for strings in [
        &f.backends,
        &f.program_elf_sha256,
        &f.model_ids,
        &f.profile_ids,
    ] {
        ensure!(
            strings.len() <= MAX_REPORT_CONFIG_ITEMS,
            "filter count limit exceeded"
        );
        let mut seen = BTreeSet::new();
        for s in strings {
            ensure!(
                !s.is_empty() && s.len() <= 1024 && seen.insert(s),
                "empty, long or duplicate filter"
            );
        }
    }
    let mut names = BTreeSet::new();
    let mut previous_max = Some(0);
    for (index, bucket) in config.buckets.iter().enumerate() {
        ensure!(
            !bucket.name.trim().is_empty()
                && bucket.name.len() <= 128
                && names.insert(&bucket.name),
            "empty, long or duplicate bucket name"
        );
        ensure!(
            bucket
                .maximum_exclusive
                .is_none_or(|max| bucket.minimum < max),
            "bucket requires minimum < maximum"
        );
        ensure!(
            index == 0 || previous_max.is_some_and(|max| bucket.minimum >= max),
            "buckets must be ordered and non-overlapping"
        );
        previous_max = bucket.maximum_exclusive;
    }
    let mut seen = BTreeSet::new();
    for q in &config.quantiles {
        ensure!(seen.insert(quantile(q)?), "duplicate numeric quantile");
    }
    ensure!(
        canonical_json(config)?.len() as u64 <= MAX_REPORT_CONFIG_BYTES,
        "report config byte limit exceeded"
    );
    Ok(())
}

/// All absent dimensions are unknown: a constrained filter cannot select an unknown value.
/// Unconstrained rows still enter selected_counts even when their category is excluded from math.
fn matches_filters(result: &ComparisonResultV1, filters: &ReportFiltersV1) -> bool {
    let (quote, observation, profile) = match result {
        ComparisonResultV1::Joined {
            quote,
            observation,
            profile_id,
            ..
        } => (
            Some(quote.as_ref()),
            Some(observation),
            Some(profile_id.as_str()),
        ),
        ComparisonResultV1::IncompatibleProfile {
            quote, observation, ..
        }
        | ComparisonResultV1::AmbiguousProfile {
            quote, observation, ..
        } => (Some(quote.as_ref()), Some(observation), None),
        ComparisonResultV1::UnmatchedQuote { quote, .. } => (Some(quote.as_ref()), None, None),
        ComparisonResultV1::UnmatchedObservation { observation, .. } => {
            (None, Some(observation), None)
        }
        ComparisonResultV1::InvalidQuote { .. } | ComparisonResultV1::InvalidObservation { .. } => {
            (None, None, None)
        }
    };
    (filters.chain_ids.is_empty()
        || quote.is_some_and(|q| filters.chain_ids.contains(&q.proposal.chain.chain_id)))
        && (filters.backends.is_empty()
            || observation.is_some_and(|o| filters.backends.contains(&o.backend)))
        && (filters.program_elf_sha256.is_empty()
            || observation
                .is_some_and(|o| filters.program_elf_sha256.contains(&o.program.elf_sha256)))
        && (filters.model_ids.is_empty()
            || quote.is_some_and(|q| filters.model_ids.contains(&q.quote.model_id)))
        && (filters.profile_ids.is_empty()
            || profile.is_some_and(|p| filters.profile_ids.iter().any(|id| id == p)))
}

fn stratum(result: &ComparisonResultV1) -> Result<Option<(StratumIdentityV1, (u128, u128))>> {
    let (quote, observation, profile_id, compatible, normalization) = match result {
        ComparisonResultV1::Joined {
            quote,
            observation,
            profile_id,
            normalization,
            ..
        } => (
            quote,
            observation,
            Some(profile_id.clone()),
            true,
            normalization,
        ),
        ComparisonResultV1::IncompatibleProfile {
            quote,
            observation,
            normalization,
            ..
        } => (quote, observation, None, false, normalization),
        _ => return Ok(None),
    };
    let fork_topology = quote
        .proposal
        .fork_segments
        .iter()
        .map(|segment| {
            Ok(ForkTopologySegmentV1 {
                fork: segment.fork.clone(),
                first_block_offset: segment
                    .first_block
                    .checked_sub(quote.proposal.first_block)
                    .context("fork segment begins before proposal")?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Some((
        StratumIdentityV1 {
            chain_id: quote.proposal.chain.chain_id,
            fork_topology,
            backend: observation.backend.clone(),
            workload_unit: observation.workload.unit.clone(),
            measurement_stage: observation.measurement_stage.clone(),
            program: observation.program.clone(),
            backend_version: observation.backend_version.clone(),
            model_id: quote.quote.model_id.clone(),
            profile_id,
            compatible,
            normalization_sha256: normalization
                .as_ref()
                .map(|n| n.normalization_sha256.clone()),
        },
        (quote.quote.prove.total_pgu, observation.workload.value),
    )))
}

fn rational_ratio(value: &BigRational) -> Result<RatioV1> {
    crate::arithmetic::ratio(
        &value
            .numer()
            .to_biguint()
            .context("negative metric numerator")?,
        &value
            .denom()
            .to_biguint()
            .context("negative metric denominator")?,
    )
}

/// Round n/sqrt(d) directly using integer midpoint comparisons. No approximate sqrt or float:
/// floor(sqrt(floor(n²*10^12/d))) is the exact six-place floor, then ties round to even.
fn sqrt_ratio_display(numerator: &BigInt, denominator: &BigUint) -> Result<String> {
    ensure!(!denominator.is_zero(), "zero correlation denominator");
    let magnitude = numerator
        .abs()
        .to_biguint()
        .context("correlation magnitude")?;
    let scale = BigUint::from(1_000_000u32);
    let square = &magnitude * &magnitude * &scale * &scale;
    let mut rounded = (&square / denominator).sqrt();
    let midpoint = (&rounded * 2u32 + 1u32).pow(2) * denominator;
    let four_square = &square * 4u32;
    if four_square > midpoint || (four_square == midpoint && (&rounded % 2u32).is_one()) {
        rounded += 1u32;
    }
    let sign = if numerator.is_negative() && !rounded.is_zero() {
        "-"
    } else {
        ""
    };
    Ok(format!(
        "{sign}{}.{:0>6}",
        &rounded / &scale,
        (&rounded % &scale).to_string()
    ))
}
fn diagnostic(
    method: &str,
    numerator: BigInt,
    denominator: BigUint,
    count: usize,
) -> Result<MetricDiagnosticV1> {
    let unavailable_reason = if count < 2 {
        Some("requires at least two points".into())
    } else if denominator.is_zero() {
        Some("constant series or all tied pairs; zero variance denominator".into())
    } else {
        None
    };
    Ok(MetricDiagnosticV1 {
        method: method.into(),
        value: if unavailable_reason.is_none() {
            Some(sqrt_ratio_display(&numerator, &denominator)?)
        } else {
            None
        },
        unavailable_reason,
    })
}
fn pearson(points: &[(u128, u128)], method: &str) -> Result<MetricDiagnosticV1> {
    let n = BigInt::from(points.len());
    let (mut sx, mut sy, mut sxx, mut syy, mut sxy) = (
        BigInt::zero(),
        BigInt::zero(),
        BigInt::zero(),
        BigInt::zero(),
        BigInt::zero(),
    );
    for &(x, y) in points {
        let x = BigInt::from(x);
        let y = BigInt::from(y);
        sxx += &x * &x;
        syy += &y * &y;
        sxy += &x * &y;
        sx += x;
        sy += y;
    }
    let covariance = &n * sxy - &sx * &sy;
    let variance = (&n * sxx - &sx * &sx) * (&n * syy - &sy * &sy);
    diagnostic(
        method,
        covariance,
        variance.to_biguint().context("negative variance")?,
        points.len(),
    )
}
fn double_ranks(values: impl Iterator<Item = u128>) -> Vec<u128> {
    let mut sorted: Vec<_> = values.enumerate().map(|(i, value)| (value, i)).collect();
    sorted.sort_unstable(); // Original index provides a deterministic stable tie order.
    let mut ranks = vec![0; sorted.len()];
    let mut begin = 0;
    while begin < sorted.len() {
        let mut end = begin + 1;
        while end < sorted.len() && sorted[end].0 == sorted[begin].0 {
            end += 1;
        }
        let rank = (begin + 1 + end) as u128; // Twice the average of one-based first/last ranks.
        for &(_, index) in &sorted[begin..end] {
            ranks[index] = rank;
        }
        begin = end;
    }
    ranks
}
fn correlations(points: &[(u128, u128)]) -> Result<BTreeMap<String, MetricDiagnosticV1>> {
    let mut diagnostics = BTreeMap::new();
    diagnostics.insert(
        "raw_value_pearson_diagnostic".into(),
        pearson(
            points,
            "raw-value-pearson-correlation;exact-integer-moments;sqrt-rational-half-even-6",
        )?,
    );
    let rx = double_ranks(points.iter().map(|p| p.0));
    let ry = double_ranks(points.iter().map(|p| p.1));
    let ranks: Vec<_> = rx.into_iter().zip(ry).collect();
    diagnostics.insert("raw_value_spearman_diagnostic".into(), pearson(&ranks, "raw-value-spearman-correlation;stable-average-tie-ranks;exact-doubled-ranks;sqrt-rational-half-even-6")?);
    let mut concordance = 0i64;
    let mut ties_x = 0u64;
    let mut ties_y = 0u64;
    for (i, &(x, y)) in points.iter().enumerate() {
        for &(xx, yy) in &points[i + 1..] {
            use std::cmp::Ordering::*;
            let cx = x.cmp(&xx);
            let cy = y.cmp(&yy);
            ties_x += u64::from(cx == Equal);
            ties_y += u64::from(cy == Equal);
            if cx != Equal && cy != Equal {
                concordance += if cx == cy { 1 } else { -1 };
            }
        }
    }
    let pairs = (points.len() as u64) * (points.len().saturating_sub(1) as u64) / 2;
    let denominator = BigUint::from(pairs - ties_x) * BigUint::from(pairs - ties_y);
    diagnostics.insert("raw_value_kendall_tau_b_diagnostic".into(), diagnostic("raw-value-kendall-tau-b-correlation;exact-pair-counts;ties-in-both-marginals;sqrt-rational-half-even-6", concordance.into(), denominator, points.len())?);
    Ok(diagnostics)
}

fn aggregate(
    identity: StratumIdentityV1,
    points: &[(u128, u128)],
    config: &ReportConfigV1,
) -> Result<StratumReportV1> {
    let (mut prediction_sum, mut actual_sum) = (BigUint::zero(), BigUint::zero());
    let mut ratios = Vec::new();
    let mut ratio_sum = BigRational::zero();
    let (mut zeros, mut infinite, mut underquote) = (0u64, 0u64, 0u64);
    for &(p, a) in points {
        prediction_sum += p;
        actual_sum += a;
        if identity.compatible {
            if a == 0 {
                zeros += 1;
                continue;
            }
            infinite += u64::from(p == 0);
            underquote += u64::from(p < a);
            let r = BigRational::new(BigInt::from(p), BigInt::from(a));
            ratio_sum += &r;
            ratios.push(r);
        }
    }
    ratios.sort();
    let eligible_count = ratios.len() as u64;
    let mut quantiles = BTreeMap::new();
    if !ratios.is_empty() {
        for q in &config.quantiles {
            let rank = quantile(q)? * BigInt::from(ratios.len());
            let index = ((rank.numer() + rank.denom() - 1u32) / rank.denom())
                .to_usize()
                .context("quantile rank overflow")?
                .max(1)
                - 1;
            quantiles.insert(q.clone(), rational_ratio(&ratios[index])?);
        }
    }
    let mut diagnostics = correlations(points)?;
    if !identity.compatible {
        diagnostics.insert(
            "direct_metrics_unavailable".into(),
            MetricDiagnosticV1 {
                method: "direct-metrics-require-compatible-profile-v1".into(),
                value: None,
                unavailable_reason: Some(
                    "incompatible profile; raw series shape diagnostics only".into(),
                ),
            },
        );
    } else {
        if eligible_count == 0 {
            diagnostics.insert("per_row_metrics_unavailable".into(), MetricDiagnosticV1 { method: "count-weighted-ratio;underquote-rate;nearest-rank-quantiles;compatible-positive-actual-population".into(), value: None, unavailable_reason: Some("no compatible rows with positive actual; per-row ratios, quantiles and rate unavailable".into()) });
        }
        if actual_sum.is_zero() {
            diagnostics.insert(
                "workload_weighted_ratio_unavailable".into(),
                MetricDiagnosticV1 {
                    method: "workload-weighted-prediction-over-actual;full-sums".into(),
                    value: None,
                    unavailable_reason: Some("full actual sum is zero".into()),
                },
            );
        }
    }
    let mut buckets = Vec::new();
    for bucket in &config.buckets {
        let actual: Vec<_> = points
            .iter()
            .filter(|(p, _)| {
                *p >= bucket.minimum && bucket.maximum_exclusive.is_none_or(|max| *p < max)
            })
            .map(|(_, a)| *a)
            .collect();
        let positive: Vec<_> = actual.iter().copied().filter(|a| *a > 0).collect();
        let reason = if actual.contains(&0) {
            Some("selected bucket contains zero actual; amplification unavailable".into())
        } else if positive.len() < 2 {
            Some("requires at least two comparable positive points".into())
        } else {
            None
        };
        let amplification = if reason.is_none() {
            Some(crate::arithmetic::ratio(
                &BigUint::from(*positive.iter().max().context("positive maximum")?),
                &BigUint::from(*positive.iter().min().context("positive minimum")?),
            )?)
        } else {
            None
        };
        diagnostics.insert(format!("bucket:{}:amplification_diagnostic", bucket.name), MetricDiagnosticV1 { method: "prediction-range-bucket-raw-actual-amplification;max(actual)/min(positive-actual);any-zero-or-fewer-than-two-positive-unavailable;half-even-6".into(), value: amplification.as_ref().map(|r| r.display.clone()), unavailable_reason: reason });
        buckets.push(BucketSummaryV1 {
            name: bucket.name.clone(),
            count: actual.len() as u64,
            amplification,
        });
    }
    Ok(StratumReportV1 {
        count_weighted_prediction_over_actual: (eligible_count > 0).then(|| rational_ratio(&(ratio_sum / BigInt::from(eligible_count)))).transpose()?,
        workload_weighted_prediction_over_actual: (identity.compatible && !actual_sum.is_zero()).then(|| crate::arithmetic::ratio(&prediction_sum, &actual_sum)).transpose()?,
        underquote_rate: (eligible_count > 0).then(|| crate::arithmetic::ratio(&underquote.into(), &eligible_count.into())).transpose()?,
        identity, row_count: points.len() as u64, eligible_count, zero_actual_count: zeros, infinite_underpricing_count: infinite,
        prediction_sum: prediction_sum.to_string(), actual_sum: actual_sum.to_string(),
        quantile_method: "nearest-rank;max(1,ceil(q*n));population=compatible-positive-actual-including-zero-prediction;count-weighted-and-underquote-use-same-population;workload-weighted-uses-full-sums".into(),
        quantiles, diagnostics, buckets,
    })
}

/// Typed callers bind canonical config bytes. The CLI binds exact raw config bytes through the
/// same builder, so whitespace in an input file is retained as provenance without changing math.
pub fn build_report(
    run: &VerifiedComparisonRun,
    config: &ReportConfigV1,
    executable: &ExecutableIdentity,
) -> Result<ReportV1> {
    validate_config(config)?;
    build_report_with_hash(
        run,
        config,
        executable,
        sha256_hex(&canonical_json(config)?),
    )
}
fn build_report_with_hash(
    run: &VerifiedComparisonRun,
    config: &ReportConfigV1,
    executable: &ExecutableIdentity,
    config_sha256: String,
) -> Result<ReportV1> {
    validate_config(config)?;
    let mut report = ReportV1 {
        schema_version: 1,
        report_id: String::new(),
        comparison_manifest_sha256: run.manifest_sha256.clone(),
        row_sequence_sha256: run.manifest.row_sequence_sha256.clone(),
        config_sha256,
        executable: executable.clone(),
        arithmetic_policy: format!("{ARITHMETIC_POLICY};{METRIC_POLICY}"),
        status: run.manifest.status,
        whole_run_counts: run.manifest.counts.clone(),
        selected_counts: BTreeMap::new(),
        strata: vec![],
        excluded: vec![],
    };
    let mut groups: BTreeMap<StratumIdentityV1, Vec<(u128, u128)>> = BTreeMap::new();
    for row in &run.rows {
        if !matches_filters(&row.result, &config.filters) {
            report.excluded.push(ExclusionV1 {
                comparison_row_id: row.comparison_row_id.clone(),
                reason: "filtered_out;missing-or-nonmatching-filter-dimension".into(),
            });
            continue;
        }
        *report
            .selected_counts
            .entry(comparison_kind(&row.result))
            .or_default() += 1;
        if let Some((identity, point)) = stratum(&row.result)? {
            groups.entry(identity).or_default().push(point);
        } else {
            let reason = match &row.result {
                ComparisonResultV1::AmbiguousProfile { .. } => "ambiguous_profile",
                ComparisonResultV1::UnmatchedQuote { .. } => "unmatched_quote",
                ComparisonResultV1::UnmatchedObservation { .. } => "unmatched_observation",
                ComparisonResultV1::InvalidQuote { .. } => "invalid_quote",
                ComparisonResultV1::InvalidObservation { .. } => "invalid_observation",
                _ => unreachable!(),
            };
            report.excluded.push(ExclusionV1 {
                comparison_row_id: row.comparison_row_id.clone(),
                reason: reason.into(),
            });
        }
    }
    let mut output_bytes = canonical_json(&report)?.len() as u64;
    for (identity, points) in groups {
        let stratum = aggregate(identity, &points, config)?;
        output_bytes += canonical_json(&stratum)?.len() as u64 + 1;
        ensure!(
            output_bytes <= MAX_REPORT_OUTPUT_BYTES - 128,
            "report output byte limit exceeded"
        );
        report.strata.push(stratum);
    }
    report.report_id = content_id(
        "zkgas-report-v1",
        &(
            &report.comparison_manifest_sha256,
            &report.row_sequence_sha256,
            &report.config_sha256,
            executable,
            &report.arithmetic_policy,
            &config.filters,
            GROUPING_POLICY,
        ),
    )?;
    Ok(report)
}

pub fn run_report(args: &ReportArgs, executable: &ExecutableIdentity) -> Result<ReportRun> {
    let run = verify_comparison_run(&args.comparisons)?;
    let raw = read_bounded(&args.config, MAX_REPORT_CONFIG_BYTES)?;
    let config = parse_config(&raw).context("strict bounded report config")?;
    let report = build_report_with_hash(&run, &config, executable, sha256_hex(&raw))?;
    let bytes = canonical_json(&report)?;
    ensure!(
        bytes.len() as u64 <= MAX_REPORT_OUTPUT_BYTES,
        "report output byte limit exceeded"
    );
    let commit = commit_file_bounded(&args.output, &bytes, MAX_REPORT_OUTPUT_BYTES)?;
    Ok(ReportRun { report, commit })
}
