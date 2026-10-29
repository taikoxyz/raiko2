//! Deterministic file-only proposal replay and quote envelopes.
use std::{
    collections::BTreeSet,
    fs::File,
    io::Read,
    path::{Component, Path, PathBuf},
    time::{Duration, Instant},
};

use alethia_reth_chainspec::spec::TaikoExecutorSpec;
use anyhow::{Context, Result, ensure};
use bincode::Options;
use clap::Args;
use raiko2_primitives_shasta::GuestInput;
use raiko2_zkgas_trace::{
    ExecutionProfile, ProposalWorkloadQuote, quote_shasta_proposal_with_execution_profile,
};
#[cfg(not(target_os = "linux"))]
use rustix::fs::openat;
#[cfg(target_os = "linux")]
use rustix::fs::{CWD, ResolveFlags, openat2};
use rustix::fs::{Mode, OFlags};

use crate::{
    contracts::*,
    identity::{canonical_json, content_id, sha256_hex},
    output::{AtomicFileWriter, CommitOutcome},
};

pub const MAX_QUOTE_OUTPUT_BYTES: u64 = 64 * 1024 * 1024;

/// Replay is deliberately sequential, with a bounded input working set.
/// Defaults are contract hard ceilings; callers may only tighten them.
#[derive(Clone, Debug, Args, serde::Serialize)]
pub struct QuoteLimits {
    #[arg(long, default_value_t = 16 * 1024 * 1024)]
    pub max_manifest_bytes: u64,
    #[arg(long, default_value_t = 512 * 1024 * 1024)]
    pub max_input_bytes: u64,
    #[arg(long, default_value_t = 8 * 1024 * 1024 * 1024)]
    pub max_total_bytes: u64,
    #[arg(long, default_value_t = 10_000)]
    pub max_rows: usize,
    /// Checked before and after each native replay; an in-progress replay is not interrupted.
    #[arg(long, default_value_t = 3_600)]
    pub max_elapsed_seconds: u64,
    /// Maximum complete JSONL output; may only tighten the downstream-compatible hard ceiling.
    #[arg(long, default_value_t = MAX_QUOTE_OUTPUT_BYTES)]
    pub max_output_bytes: u64,
    /// Native replay is memory intensive; only sequential execution is supported.
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u8).range(1..=1))]
    pub concurrency: u8,
}
impl Default for QuoteLimits {
    fn default() -> Self {
        Self {
            max_manifest_bytes: 16 * 1024 * 1024,
            max_input_bytes: 512 * 1024 * 1024,
            max_total_bytes: 8 * 1024 * 1024 * 1024,
            max_rows: 10_000,
            max_elapsed_seconds: 3_600,
            max_output_bytes: MAX_QUOTE_OUTPUT_BYTES,
            concurrency: 1,
        }
    }
}
impl QuoteLimits {
    fn validate(&self) -> Result<()> {
        let ceilings = Self::default();
        for (field, requested, ceiling) in [
            (
                "max_manifest_bytes",
                self.max_manifest_bytes,
                ceilings.max_manifest_bytes,
            ),
            (
                "max_input_bytes",
                self.max_input_bytes,
                ceilings.max_input_bytes,
            ),
            (
                "max_total_bytes",
                self.max_total_bytes,
                ceilings.max_total_bytes,
            ),
            (
                "max_elapsed_seconds",
                self.max_elapsed_seconds,
                ceilings.max_elapsed_seconds,
            ),
            (
                "max_output_bytes",
                self.max_output_bytes,
                ceilings.max_output_bytes,
            ),
        ] {
            ensure!(
                requested <= ceiling,
                "{field} exceeds hard ceiling {ceiling}"
            );
        }
        ensure!(
            self.max_rows <= ceilings.max_rows,
            "max_rows exceeds hard ceiling {}",
            ceilings.max_rows
        );
        ensure!(self.concurrency == 1, "concurrency must be one");
        Ok(())
    }
}

#[derive(Clone, Debug, Default, clap::Parser)]
#[command(about = "Quote current Shasta GuestInput files without fetching data or running guests")]
#[command(group(clap::ArgGroup::new("input").required(true).args(["input_bincode", "input_json", "manifest"])))]
pub struct QuoteArgs {
    /// Offline replay policy; EVM gas-only mode is an explicitly labelled experiment.
    #[arg(long, default_value = "legacy-unzen")]
    pub execution_profile: ExecutionProfile,
    #[arg(long)]
    pub input_bincode: Option<PathBuf>,
    /// Developer convenience; parsed exactly as supplied, without proof-carry backfill.
    #[arg(long)]
    pub input_json: Option<PathBuf>,
    #[arg(long, conflicts_with_all = ["corpus_id", "sample_id"])]
    pub manifest: Option<PathBuf>,
    #[arg(long, required_unless_present = "manifest")]
    pub corpus_id: Option<String>,
    #[arg(long, required_unless_present = "manifest")]
    pub sample_id: Option<String>,
    #[arg(long)]
    pub output: PathBuf,
    #[command(flatten)]
    pub limits: QuoteLimits,
}

#[derive(Debug)]
pub struct QuoteRun {
    pub status: RunStatus,
    pub row_count: usize,
    pub commit: CommitOutcome,
}
impl QuoteRun {
    pub fn exit_code(&self) -> i32 {
        match self.status {
            RunStatus::Complete => 0,
            RunStatus::Partial => 2,
        }
    }
}

fn failed(code: &str, message: impl ToString) -> FailureV1 {
    FailureV1 {
        code: code.into(),
        message: message.to_string(),
    }
}
type InputResult<T> = std::result::Result<T, FailureV1>;

struct InputIdentity {
    corpus_id: String,
    sample_id: String,
    verified_key: Option<QuoteKey>,
    canonical_byte_length: Option<u64>,
    observed_sha256: Option<String>,
    observed_byte_length: Option<u64>,
}
impl InputIdentity {
    fn new(corpus_id: String, sample_id: String) -> Self {
        Self {
            corpus_id,
            sample_id,
            verified_key: None,
            canonical_byte_length: None,
            observed_sha256: None,
            observed_byte_length: None,
        }
    }
    fn observe(&mut self, bytes: &[u8]) {
        self.observed_sha256 = Some(sha256_hex(bytes));
        self.observed_byte_length = Some(bytes.len() as u64);
    }
    fn verify(&mut self, canonical: &[u8]) {
        self.verified_key = Some(QuoteKey {
            corpus_id: self.corpus_id.clone(),
            sample_id: self.sample_id.clone(),
            guest_input_sha256: sha256_hex(canonical),
        });
        self.canonical_byte_length = Some(canonical.len() as u64);
    }
}

fn read_bounded(file: File, limit: u64) -> InputResult<Vec<u8>> {
    let metadata = file.metadata().map_err(|e| failed("input_io", e))?;
    if !metadata.is_file() {
        return Err(failed("input_not_regular", "input must be a regular file"));
    }
    if metadata.len() > limit {
        return Err(failed("input_byte_limit", "input exceeds byte limit"));
    }
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|e| failed("input_io", e))?;
    if bytes.len() as u64 > limit {
        return Err(failed("input_byte_limit", "input exceeds byte limit"));
    }
    Ok(bytes)
}

#[cfg(target_os = "linux")]
fn open_path_no_symlinks(path: &Path, flags: OFlags) -> Result<File> {
    Ok(File::from(openat2(
        CWD,
        path,
        flags | OFlags::NOFOLLOW,
        Mode::empty(),
        ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )?))
}

/// Component-wise fallback for platforms without `openat2`. Every ancestor is opened and pinned
/// before the next component, and `NOFOLLOW` rejects a symlink at every hop.
#[cfg(not(target_os = "linux"))]
fn open_path_no_symlinks(path: &Path, flags: OFlags) -> Result<File> {
    let mut directory = if path.is_absolute() {
        File::open("/")?
    } else {
        File::open(".")?
    };
    let mut components = path
        .components()
        .filter_map(|component| match component {
            Component::RootDir | Component::CurDir => None,
            Component::ParentDir => Some(std::ffi::OsStr::new("..")),
            Component::Normal(name) => Some(name),
            Component::Prefix(_) => None,
        })
        .peekable();
    while let Some(component) = components.next() {
        let component_flags = if components.peek().is_none() {
            flags
        } else {
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY
        };
        directory = File::from(openat(
            &directory,
            component,
            component_flags | OFlags::NOFOLLOW,
            Mode::empty(),
        )?);
    }
    Ok(directory)
}

#[cfg(target_os = "linux")]
fn open_beneath_no_symlinks(directory: &File, path: &Path, flags: OFlags) -> Result<File> {
    Ok(File::from(openat2(
        directory,
        path,
        flags | OFlags::NOFOLLOW,
        Mode::empty(),
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )?))
}

#[cfg(not(target_os = "linux"))]
fn open_beneath_no_symlinks(directory: &File, path: &Path, flags: OFlags) -> Result<File> {
    let mut directory = directory.try_clone()?;
    let mut components = path
        .components()
        .filter_map(|component| match component {
            Component::CurDir => None,
            Component::Normal(name) => Some(name),
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => None,
        })
        .peekable();
    while let Some(component) = components.next() {
        let component_flags = if components.peek().is_none() {
            flags
        } else {
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY
        };
        directory = File::from(openat(
            &directory,
            component,
            component_flags | OFlags::NOFOLLOW,
            Mode::empty(),
        )?);
    }
    Ok(directory)
}

pub(crate) fn open_regular_no_symlinks(path: &Path) -> Result<File> {
    open_path_no_symlinks(path, OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NONBLOCK)
}

fn open_single(path: &Path) -> InputResult<File> {
    open_regular_no_symlinks(path)
        .map_err(|_| failed("input_open", "cannot open input without following symlinks"))
}

/// Resolve every component in the kernel, anchored to an already opened manifest directory.
/// NONBLOCK prevents opening a FIFO from hanging before its descriptor can be rejected.
pub fn open_manifest_input(manifest_dir: &File, relative: &Path) -> Result<File> {
    manifest_file(manifest_dir, relative)
        .map_err(|failure| anyhow::anyhow!("{}: {}", failure.code, failure.message))
}

fn manifest_file(manifest_dir: &File, path: &Path) -> InputResult<File> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(failed(
            "input_path",
            "manifest input path must stay beneath its directory",
        ));
    }
    let file = open_beneath_no_symlinks(
        manifest_dir,
        path,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
    )
    .map_err(|_| {
        failed(
            "input_open",
            "cannot open manifest input without following symlinks",
        )
    })?;
    if !file
        .metadata()
        .map_err(|e| failed("input_io", e))?
        .is_file()
    {
        return Err(failed("input_not_regular", "input must be a regular file"));
    }
    Ok(file)
}

fn decode(bytes: &[u8], json: bool) -> InputResult<(GuestInput, Vec<u8>)> {
    let input: GuestInput = if json {
        serde_json::from_slice(bytes).map_err(|e| failed("input_decode", e))?
    } else {
        bincode::DefaultOptions::new()
            .with_fixint_encoding()
            .allow_trailing_bytes()
            .with_limit(bytes.len() as u64)
            .deserialize(bytes)
            .map_err(|e| failed("input_decode", e))?
    };
    let canonical = bincode::serialize(&input).map_err(|e| failed("input_canonicalization", e))?;
    if !json && canonical != bytes {
        return Err(failed(
            "noncanonical_bincode",
            "bincode does not round-trip byte-identically",
        ));
    }
    Ok((input, canonical))
}

fn proposal_identity(input: &GuestInput) -> InputResult<ProposalIdentityV1> {
    let chain = &input.taiko.chain_spec;
    let carry = &input.proof_carry_data;
    if input.taiko.proposal_id != input.taiko.proposal_event.proposal.id.to::<u64>()
        || carry.transition_input.proposal_id != input.taiko.proposal_id
        || carry.chain_id != chain.chain_id
    {
        return Err(failed(
            "proposal_identity_mismatch",
            "manifest, event and proof-carry identities disagree",
        ));
    }
    let first = input
        .witnesses
        .first()
        .ok_or_else(|| failed("empty_proposal", "proposal has no blocks"))?;
    // Proposal replay constructs its runtime from the chain ID, ignoring host fork overlays.
    // Hash those supplied overlays below, but label the fork actually executed by the core.
    let runtime = raiko2_primitives::chain_spec::builtin_taiko_chain_spec(chain.chain_id)
        .map_err(|e| failed("active_fork", e))?;
    let mut forks: Vec<ForkSegmentV1> = Vec::new();
    let mut hashes = Vec::with_capacity(input.witnesses.len());
    for (index, witness) in input.witnesses.iter().enumerate() {
        if witness.chain_spec.chain_id != chain.chain_id
            || witness.chain_spec.name != chain.name
            || witness.chain_spec.is_taiko != chain.is_taiko
        {
            return Err(failed(
                "chain_identity_mismatch",
                "witness chain differs from manifest chain",
            ));
        }
        let number = witness.block.header.number;
        if first.block.header.number.checked_add(index as u64) != Some(number) {
            return Err(failed(
                "block_range_mismatch",
                "proposal block numbers are not consecutive",
            ));
        }
        let timestamp = witness.block.header.timestamp;
        let fork = if runtime.is_unzen_active(timestamp) {
            "UNZEN"
        } else if runtime.is_shasta_active(timestamp) {
            "SHASTA"
        } else {
            return Err(failed(
                "active_fork",
                "block predates the supported Shasta runtime",
            ));
        }
        .to_owned();
        match forks.last_mut() {
            Some(segment) if segment.fork == fork => segment.last_block = number,
            _ => forks.push(ForkSegmentV1 {
                fork,
                first_block: number,
                last_block: number,
            }),
        }
        hashes.push(sha256_hex(
            &canonical_json(&witness.chain_spec).map_err(|e| failed("chain_spec_identity", e))?,
        ));
    }
    Ok(ProposalIdentityV1 {
        proposal_id: input.taiko.proposal_id,
        chain: ChainIdentityV1 {
            name: chain.name.clone(),
            chain_id: chain.chain_id,
        },
        proof_carry_chain_id: carry.chain_id,
        proof_carry_proposal_id: carry.transition_input.proposal_id,
        witness_chain_spec_sha256: hashes,
        first_block: first.block.header.number,
        last_block: input
            .witnesses
            .last()
            .expect("nonempty witnesses")
            .block
            .header
            .number,
        block_count: input.witnesses.len() as u64,
        public_output: String::new(),
        fork_segments: forks,
    })
}

fn quote_input(
    input: &GuestInput,
    started: Instant,
    limits: &QuoteLimits,
    execution_profile: ExecutionProfile,
) -> InputResult<(ProposalIdentityV1, ProposalWorkloadQuote)> {
    let mut proposal = proposal_identity(input)?;
    if started.elapsed() >= Duration::from_secs(limits.max_elapsed_seconds) {
        return Err(failed(
            "elapsed_time_limit",
            "elapsed replay budget exhausted",
        ));
    }
    let result = quote_shasta_proposal_with_execution_profile(input, execution_profile);
    if started.elapsed() >= Duration::from_secs(limits.max_elapsed_seconds) {
        return Err(failed(
            "elapsed_time_limit",
            "elapsed replay budget exhausted",
        ));
    }
    let quote = result.map_err(|e| failed("proposal_quote", e))?;
    if quote.blocks.len() != input.witnesses.len() || quote.model_id.is_empty() {
        return Err(failed(
            "quote_identity_mismatch",
            "quote block count or model identity is invalid",
        ));
    }
    proposal.public_output = quote.public_output.to_string();
    Ok((proposal, quote))
}

/// Errors mean no valid requested output was committed; row failures commit a partial run.
pub fn run_quote(args: &QuoteArgs, executable: &ExecutableIdentity) -> Result<QuoteRun> {
    args.limits.validate()?;
    let started = Instant::now();
    ensure!(
        [
            args.input_bincode.is_some(),
            args.input_json.is_some(),
            args.manifest.is_some()
        ]
        .into_iter()
        .filter(|v| *v)
        .count()
            == 1,
        "exactly one input mode is required"
    );
    if let Some(path) = &args.manifest {
        ensure!(
            args.corpus_id.is_none() && args.sample_id.is_none(),
            "manifest supplies corpus and sample IDs"
        );
        return run_manifest(args, executable, path, started);
    }
    ensure!(args.limits.max_rows >= 1, "single input exceeds row limit");
    let corpus_id = args
        .corpus_id
        .as_ref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("single input requires a corpus ID"))?;
    let sample_id = args
        .sample_id
        .as_ref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("single input requires a sample ID"))?;
    let path = args
        .input_bincode
        .as_ref()
        .or(args.input_json.as_ref())
        .expect("single input");
    let mut identity = InputIdentity::new(corpus_id.clone(), sample_id.clone());
    let result = (|| {
        let bytes = read_bounded(
            open_single(path)?,
            args.limits.max_input_bytes.min(args.limits.max_total_bytes),
        )
        .map_err(|failure| {
            if failure.code == "input_byte_limit"
                && args.limits.max_total_bytes < args.limits.max_input_bytes
            {
                failed("total_byte_limit", "aggregate input bytes exceed limit")
            } else {
                failure
            }
        })?;
        identity.observe(&bytes);
        let (input, canonical) = decode(&bytes, args.input_json.is_some())?;
        identity.verify(&canonical);
        quote_input(&input, started, &args.limits, args.execution_profile)
    })();
    #[derive(serde::Serialize)]
    struct SingleDescriptor<'a> {
        schema_version: u32,
        corpus_id: &'a str,
        sample_id: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        verified_key: Option<&'a QuoteKey>,
        #[serde(skip_serializing_if = "Option::is_none")]
        canonical_byte_length: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        observed_guest_input_sha256: Option<&'a str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        observed_guest_input_byte_length: Option<u64>,
        limits: &'a QuoteLimits,
        #[serde(skip_serializing_if = "execution_profile_is_legacy")]
        execution_profile: ExecutionProfile,
    }
    let source = InputSourceV1::Single {
        descriptor_sha256: content_id(
            "zkgas-single-input-v1",
            &SingleDescriptor {
                schema_version: 1,
                corpus_id,
                sample_id,
                verified_key: identity.verified_key.as_ref(),
                canonical_byte_length: identity.canonical_byte_length,
                observed_guest_input_sha256: if identity.verified_key.is_none() {
                    identity.observed_sha256.as_deref()
                } else {
                    None
                },
                observed_guest_input_byte_length: if identity.verified_key.is_none() {
                    identity.observed_byte_length
                } else {
                    None
                },
                limits: &args.limits,
                execution_profile: args.execution_profile,
            },
        )?,
    };
    let row = make_row(
        identity,
        source,
        None,
        executable,
        result,
        args.execution_profile,
    );
    let partial = matches!(row, QuoteRowV1::Failed(_));
    let mut bytes = canonical_json(&row)?;
    bytes.push(b'\n');
    let mut output = AtomicFileWriter::new(&args.output, args.limits.max_output_bytes)?;
    output
        .append(&bytes)
        .context("quote output byte limit exceeded")?;
    publish(output, partial, 1)
}

fn run_manifest(
    args: &QuoteArgs,
    executable: &ExecutableIdentity,
    path: &Path,
    started: Instant,
) -> Result<QuoteRun> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let directory = open_path_no_symlinks(
        parent,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY | OFlags::NOFOLLOW,
    )
    .context("open manifest directory")?;
    let name = path.file_name().context("manifest must have a filename")?;
    let file = open_manifest_input(&directory, Path::new(name)).context("open manifest")?;
    let raw = read_bounded(file, args.limits.max_manifest_bytes)
        .map_err(|e| anyhow::anyhow!("manifest: {}: {}", e.code, e.message))?;
    let manifest: InputManifestV1 = serde_json::from_slice(&raw).context("parse input manifest")?;
    ensure!(
        !manifest.corpus_id.is_empty(),
        "manifest corpus ID must not be empty"
    );
    ensure!(
        !manifest.samples.is_empty(),
        "manifest must contain at least one sample"
    );
    ensure!(
        manifest.samples.len() <= args.limits.max_rows,
        "manifest row limit exceeded"
    );
    let mut ids = BTreeSet::new();
    for sample in &manifest.samples {
        ensure!(
            !sample.sample_id.is_empty() && ids.insert(&sample.sample_id),
            "empty or duplicate sample ID"
        );
        ensure!(
            sample.guest_input.sha256.len() == 64
                && sample
                    .guest_input
                    .sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "input SHA-256 must be 64 lowercase hex characters"
        );
    }
    let total_declared = manifest.samples.iter().try_fold(0u64, |total, s| {
        total.checked_add(s.guest_input.byte_length)
    });
    let declared_exceeded = total_declared.is_none_or(|total| total > args.limits.max_total_bytes);
    // Complete declaration preflight before any input is opened, preserving one failure result
    // per semantic row rather than aborting an otherwise structurally valid manifest.
    let preflight: Vec<_> = manifest
        .samples
        .iter()
        .map(|sample| {
            if declared_exceeded {
                Some(failed(
                    "total_byte_limit",
                    "aggregate declared inputs exceed byte limit",
                ))
            } else if sample.guest_input.byte_length > args.limits.max_input_bytes {
                Some(failed(
                    "input_byte_limit",
                    "declared input exceeds byte limit",
                ))
            } else {
                None
            }
        })
        .collect();
    let manifest_hash = sha256_hex(&raw);
    drop(raw);
    let mut output = AtomicFileWriter::new(&args.output, args.limits.max_output_bytes)?;
    let mut partial = false;
    let mut total_read = 0u64;
    let count = manifest.samples.len();
    for (ordinal, (sample, preflight)) in manifest.samples.into_iter().zip(preflight).enumerate() {
        let mut identity = InputIdentity::new(manifest.corpus_id.clone(), sample.sample_id);
        let source = InputSourceV1::Manifest {
            manifest_sha256: manifest_hash.clone(),
            row_ordinal: ordinal as u64,
        };
        let result = (|| {
            if let Some(failure) = preflight {
                return Err(failure);
            }
            if started.elapsed() >= Duration::from_secs(args.limits.max_elapsed_seconds) {
                return Err(failed(
                    "elapsed_time_limit",
                    "elapsed replay budget exhausted",
                ));
            }
            let file = manifest_file(&directory, Path::new(&sample.guest_input.path))?;
            let remaining = args.limits.max_total_bytes.saturating_sub(total_read);
            let raw = read_bounded(file, args.limits.max_input_bytes.min(remaining)).map_err(
                |failure| {
                    if failure.code == "input_byte_limit" && remaining < args.limits.max_input_bytes
                    {
                        total_read = args.limits.max_total_bytes;
                        failed("total_byte_limit", "aggregate input bytes exceed limit")
                    } else {
                        failure
                    }
                },
            )?;
            total_read += raw.len() as u64;
            identity.observe(&raw);
            if raw.len() as u64 != sample.guest_input.byte_length {
                return Err(failed(
                    "input_length_mismatch",
                    "input length differs from manifest",
                ));
            }
            if sha256_hex(&raw) != sample.guest_input.sha256 {
                return Err(failed(
                    "input_hash_mismatch",
                    "input SHA-256 differs from manifest",
                ));
            }
            let (input, canonical) = decode(&raw, false)?;
            identity.verify(&canonical);
            drop(raw);
            drop(canonical);
            quote_input(&input, started, &args.limits, args.execution_profile)
        })();
        // The closure's GuestInput and all replay scratch have been dropped before serializing
        // this row. Serialize and release the quote before opening the next input.
        let row = make_row(
            identity,
            source,
            sample.metadata,
            executable,
            result,
            args.execution_profile,
        );
        partial |= matches!(row, QuoteRowV1::Failed(_));
        let mut bytes = canonical_json(&row)?;
        bytes.push(b'\n');
        output
            .append(&bytes)
            .context("quote output byte limit exceeded")?;
    }
    publish(output, partial, count)
}

fn make_row(
    identity: InputIdentity,
    input_source: InputSourceV1,
    metadata: Option<OpaqueObject>,
    executable: &ExecutableIdentity,
    result: InputResult<(ProposalIdentityV1, ProposalWorkloadQuote)>,
    execution_profile: ExecutionProfile,
) -> QuoteRowV1 {
    match result {
        Ok((proposal, quote)) => QuoteRowV1::Complete(Box::new(CompleteQuoteV1 {
            schema_version: 1,
            execution_profile: quote.execution_profile,
            key: identity
                .verified_key
                .expect("complete quote has canonical identity"),
            executable: executable.clone(),
            input_source,
            guest_input_byte_length: identity
                .canonical_byte_length
                .expect("complete quote has canonical length"),
            metadata,
            proposal,
            prediction_unit: "sp1_pgu".into(),
            prediction_surface: "prove".into(),
            calibration_profile: quote.model_id.clone(),
            quote,
        })),
        Err(failure) => QuoteRowV1::Failed(Box::new(FailedQuoteV1 {
            schema_version: 1,
            execution_profile,
            corpus_id: identity.corpus_id,
            sample_id: identity.sample_id,
            verified_key: identity.verified_key,
            observed_guest_input_sha256: identity.observed_sha256,
            observed_guest_input_byte_length: identity.observed_byte_length,
            executable: executable.clone(),
            input_source,
            metadata,
            failure,
        })),
    }
}
fn publish(output: AtomicFileWriter, partial: bool, row_count: usize) -> Result<QuoteRun> {
    let status = if partial {
        RunStatus::Partial
    } else {
        RunStatus::Complete
    };
    let commit = output.commit()?;
    Ok(QuoteRun {
        status,
        row_count,
        commit,
    })
}
