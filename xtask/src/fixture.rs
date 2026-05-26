use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::Instant;

use alloy::hex;
use alloy::primitives::B256;
use anyhow::{Context, Result, bail, ensure};
use clap::{Args, Subcommand, ValueEnum};
use raiko2_pipeline::forks::shasta::validate_shasta_guest_input;
use raiko2_primitives::ProofType;
use raiko2_primitives_shasta::{
    DEFAULT_GUEST_INPUT_ROOT, GuestInput, build_proof_carry_data, guest_input_proposal_path,
    validate_fixture_key,
};
use raiko2_protocol_shasta::libhash::hash_shasta_subproof_input;
use raiko2_protocol_shasta::shasta::ProofCarryData;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Args)]
pub(crate) struct FixtureArgs {
    #[command(subcommand)]
    command: FixtureCommand,
}

#[derive(Subcommand)]
enum FixtureCommand {
    /// Check a fixture envelope.
    Check(FixtureCheckArgs),
    /// Generate a fixture envelope from a Shasta GuestInput JSON file.
    Generate(FixtureGenerateArgs),
}

#[derive(Args)]
struct FixtureCheckArgs {
    /// Path to a `raiko2-fixture-v1` fixture envelope.
    #[arg(long)]
    case: PathBuf,

    /// Check mode. Phase 1 supports only `open-commitment`.
    #[arg(long, value_enum, default_value_t = CheckMode::OpenCommitment)]
    mode: CheckMode,

    /// Optional path to write a JSON report.
    #[arg(long)]
    json_out: Option<PathBuf>,
}

#[derive(Args, Clone)]
struct FixtureGenerateArgs {
    /// Network fixture key under the Shasta GuestInput root.
    #[arg(long)]
    network: String,

    /// Proposal id to load from the Shasta GuestInput proposal fixtures.
    #[arg(long)]
    proposal: u64,

    /// Proof type used to derive the public input opening.
    #[arg(long, value_parser = parse_proof_type, default_value = "native")]
    proof_type: ProofType,

    /// Root directory containing Shasta GuestInput fixtures.
    #[arg(long, default_value = DEFAULT_GUEST_INPUT_ROOT)]
    guest_input_root: PathBuf,

    /// Output fixture envelope path. Defaults to the canonical test/fixtures/v1 path.
    #[arg(long)]
    output: Option<PathBuf>,

    /// Optional fixture case id. Defaults to shasta/<network>/proposal_<proposal>.
    #[arg(long)]
    case_id: Option<String>,

    /// Optional human-readable fixture description.
    #[arg(long)]
    description: Option<String>,

    /// Allow overwriting an existing fixture envelope.
    #[arg(
        long,
        value_parser = clap::builder::BoolishValueParser::new(),
        num_args = 0..=1,
        default_value = "false",
        default_missing_value = "true"
    )]
    overwrite: bool,
}

pub(crate) fn run(root: &Path, args: FixtureArgs) -> Result<()> {
    match args.command {
        FixtureCommand::Check(args) => {
            let report = check_case_file(root, &args.case, args.mode)?;
            print_report(&report);
            if let Some(path) = args.json_out {
                write_report(&path, &report)?;
            }
            if report.failure_count > 0 {
                bail!("{} fixture check(s) failed", report.failure_count);
            }
            Ok(())
        }
        FixtureCommand::Generate(args) => {
            let output = generate_case_file(root, &args)?;
            println!("generated_fixture: path={}", output.display());
            Ok(())
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum CheckMode {
    OpenCommitment,
}

impl std::fmt::Display for CheckMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            CheckMode::OpenCommitment => "open-commitment",
        })
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct FixtureEnvelope {
    pub(crate) schema: String,
    pub(crate) case_id: String,
    pub(crate) fork: String,
    pub(crate) stage: FixtureStage,
    #[serde(default)]
    pub(crate) description: Option<String>,
    pub(crate) statement: FixtureStatement,
    pub(crate) input: FixtureInput,
    pub(crate) expected: FixtureExpected,
    #[serde(default)]
    pub(crate) checks: FixtureChecks,
    #[serde(default = "default_empty_object")]
    pub(crate) providers: serde_json::Value,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FixtureStage {
    Proposal,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct FixtureStatement {
    #[serde(with = "raiko2_primitives::proof_type::lowercase")]
    pub(crate) proof_type: ProofType,
    pub(crate) public_input_kind: PublicInputKind,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct FixtureInput {
    pub(crate) kind: FixtureInputKind,
    pub(crate) path: String,
    pub(crate) sha256: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FixtureInputKind {
    ShastaGuestInput,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct FixtureExpected {
    pub(crate) commitment: B256,
    pub(crate) opening: FixtureOpening,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PublicInputKind {
    ShastaSubproofInput,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum FixtureOpening {
    ShastaProofCarryData {
        proof_carry_data: Box<ProofCarryData>,
    },
}

impl FixtureOpening {
    fn shasta_proof_carry_data(carry: ProofCarryData) -> Self {
        Self::ShastaProofCarryData {
            proof_carry_data: Box::new(carry),
        }
    }

    fn as_shasta_proof_carry_data(&self) -> &ProofCarryData {
        match self {
            Self::ShastaProofCarryData { proof_carry_data } => proof_carry_data,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct FixtureChecks {
    pub(crate) open_commitment: bool,
    pub(crate) check_provider_public_input: bool,
    pub(crate) verify_proof: bool,
}

impl Default for FixtureChecks {
    fn default() -> Self {
        Self {
            open_commitment: true,
            check_provider_public_input: false,
            verify_proof: false,
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct FixtureReport {
    schema: &'static str,
    mode: CheckMode,
    success_count: usize,
    failure_count: usize,
    elapsed_ms: u64,
    results: Vec<FixtureCheckResult>,
}

#[derive(Debug, Serialize)]
pub(crate) struct FixtureCheckResult {
    pub(crate) case_id: String,
    pub(crate) mode: CheckMode,
    pub(crate) passed: bool,
    #[serde(with = "raiko2_primitives::proof_type::lowercase")]
    pub(crate) proof_type: ProofType,
    pub(crate) commitment: B256,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<String>,
}

pub(crate) fn check_case_file(
    root: &Path,
    case_path: &Path,
    mode: CheckMode,
) -> Result<FixtureReport> {
    let started_at = Instant::now();
    let raw =
        fs::read_to_string(case_path).with_context(|| format!("read {}", case_path.display()))?;
    let envelope: FixtureEnvelope =
        serde_json::from_str(&raw).with_context(|| format!("parse {}", case_path.display()))?;

    let result = match mode {
        CheckMode::OpenCommitment => match check_open_commitment(root, &envelope) {
            Ok(result) => result,
            Err(err) => FixtureCheckResult {
                case_id: envelope.case_id.clone(),
                mode,
                passed: false,
                proof_type: envelope.statement.proof_type,
                commitment: envelope.expected.commitment,
                error: Some(format!("{err:#}")),
            },
        },
    };
    let success_count = usize::from(result.passed);
    let failure_count = usize::from(!result.passed);

    Ok(FixtureReport {
        schema: "raiko2-fixture-report-v1",
        mode,
        success_count,
        failure_count,
        elapsed_ms: elapsed_ms(started_at),
        results: vec![result],
    })
}

fn generate_case_file(root: &Path, args: &FixtureGenerateArgs) -> Result<PathBuf> {
    let guest_input_root = resolve_maybe_repo_relative_path(root, &args.guest_input_root);
    let input_path = guest_input_proposal_path(&guest_input_root, &args.network, args.proposal)?;
    let output_path = match &args.output {
        Some(path) => resolve_maybe_repo_relative_path(root, path),
        None => default_fixture_output_path(root, &args.network, args.proposal)?,
    };

    if output_path.exists() && !args.overwrite {
        bail!(
            "fixture output already exists: {} (pass --overwrite true to replace it)",
            output_path.display()
        );
    }

    let input_bytes =
        fs::read(&input_path).with_context(|| format!("read {}", input_path.display()))?;
    let mut input: GuestInput = serde_json::from_slice(&input_bytes)
        .with_context(|| format!("parse {}", input_path.display()))?;
    normalize_guest_input(&mut input);
    ensure!(
        input.taiko.proposal_id == args.proposal,
        "GuestInput proposal_id mismatch: expected {}, got {}",
        args.proposal,
        input.taiko.proposal_id
    );

    let derived_carry =
        build_proof_carry_data(&input, args.proof_type).map_err(|err| anyhow::anyhow!("{err}"))?;
    if input.proof_carry_data != ProofCarryData::default()
        && input.proof_carry_data != derived_carry
    {
        bail!("GuestInput proof_carry_data does not match derived opening");
    }
    input.proof_carry_data = derived_carry.clone();
    validate_shasta_guest_input(&input).map_err(|err| anyhow::anyhow!("{err}"))?;

    let envelope = FixtureEnvelope {
        schema: "raiko2-fixture-v1".to_string(),
        case_id: args
            .case_id
            .clone()
            .unwrap_or_else(|| format!("shasta/{}/proposal_{}", args.network, args.proposal)),
        fork: "shasta".to_string(),
        stage: FixtureStage::Proposal,
        description: args.description.clone(),
        statement: FixtureStatement {
            proof_type: args.proof_type,
            public_input_kind: PublicInputKind::ShastaSubproofInput,
        },
        input: FixtureInput {
            kind: FixtureInputKind::ShastaGuestInput,
            path: repo_relative_path_string(root, &input_path)?,
            sha256: sha256_hex(&input_bytes),
        },
        expected: FixtureExpected {
            commitment: hash_shasta_subproof_input(&derived_carry),
            opening: FixtureOpening::shasta_proof_carry_data(derived_carry),
        },
        checks: FixtureChecks::default(),
        providers: serde_json::Value::Object(Default::default()),
    };

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let contents = serde_json::to_string_pretty(&envelope).context("serialize fixture envelope")?;
    fs::write(&output_path, contents)
        .with_context(|| format!("write {}", output_path.display()))?;
    Ok(output_path)
}

pub(crate) fn check_open_commitment(
    root: &Path,
    envelope: &FixtureEnvelope,
) -> Result<FixtureCheckResult> {
    validate_envelope_header(envelope)?;
    let input_path = resolve_repo_relative_path(root, &envelope.input.path)?;
    let input_bytes =
        fs::read(&input_path).with_context(|| format!("read {}", input_path.display()))?;
    let actual_sha = sha256_hex(&input_bytes);
    ensure!(
        normalize_hex(&envelope.input.sha256) == actual_sha,
        "fixture input sha256 mismatch: expected {}, got {}",
        envelope.input.sha256,
        actual_sha
    );

    let mut input: GuestInput = serde_json::from_slice(&input_bytes)
        .with_context(|| format!("parse {}", input_path.display()))?;
    normalize_guest_input(&mut input);

    let derived_carry = build_proof_carry_data(&input, envelope.statement.proof_type)
        .map_err(|err| anyhow::anyhow!("{err}"))?;
    let opening_carry = envelope.expected.opening.as_shasta_proof_carry_data();
    ensure!(
        opening_carry == &derived_carry,
        "fixture opening does not match GuestInput-derived opening"
    );

    if input.proof_carry_data != ProofCarryData::default()
        && input.proof_carry_data != derived_carry
    {
        bail!("GuestInput proof_carry_data does not match derived opening");
    }
    input.proof_carry_data = derived_carry.clone();

    validate_shasta_guest_input(&input).map_err(|err| anyhow::anyhow!("{err}"))?;

    let commitment = hash_shasta_subproof_input(&derived_carry);
    ensure!(
        envelope.expected.commitment == commitment,
        "fixture commitment does not match opening: expected {}, got {}",
        envelope.expected.commitment,
        commitment
    );

    Ok(FixtureCheckResult {
        case_id: envelope.case_id.clone(),
        mode: CheckMode::OpenCommitment,
        passed: true,
        proof_type: envelope.statement.proof_type,
        commitment,
        error: None,
    })
}

pub(crate) fn write_report(path: &Path, report: &FixtureReport) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let contents = serde_json::to_string_pretty(report).context("serialize fixture report")?;
    fs::write(path, contents).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

fn validate_envelope_header(envelope: &FixtureEnvelope) -> Result<()> {
    ensure!(
        envelope.schema == "raiko2-fixture-v1",
        "unsupported fixture schema: {}",
        envelope.schema
    );
    ensure!(
        envelope.fork == "shasta",
        "unsupported fixture fork: {}",
        envelope.fork
    );
    ensure!(
        envelope.stage == FixtureStage::Proposal,
        "unsupported fixture stage: {:?}",
        envelope.stage
    );
    ensure!(
        envelope.input.kind == FixtureInputKind::ShastaGuestInput,
        "unsupported fixture input kind: {:?}",
        envelope.input.kind
    );
    ensure!(
        envelope.statement.public_input_kind == PublicInputKind::ShastaSubproofInput,
        "unsupported public input kind: {:?}",
        envelope.statement.public_input_kind
    );
    ensure!(
        envelope.checks.open_commitment,
        "fixture does not enable open-commitment check"
    );
    Ok(())
}

fn resolve_repo_relative_path(root: &Path, raw: &str) -> Result<PathBuf> {
    let path = Path::new(raw);
    ensure!(
        !path.is_absolute(),
        "fixture input path must be repo-relative"
    );
    ensure!(
        path.components().all(|component| !matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )),
        "fixture input path must not escape the repo root"
    );
    let canonical_root = root
        .canonicalize()
        .with_context(|| format!("canonicalize repo root {}", root.display()))?;
    let joined = root.join(path);
    let canonical_path = joined
        .canonicalize()
        .with_context(|| format!("canonicalize fixture input {}", joined.display()))?;
    ensure!(
        canonical_path.starts_with(&canonical_root),
        "fixture input path must stay within the repo root after canonicalization"
    );
    Ok(canonical_path)
}

fn resolve_maybe_repo_relative_path(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

fn default_fixture_output_path(root: &Path, network: &str, proposal_id: u64) -> Result<PathBuf> {
    let network = validate_fixture_key("network", network)?;
    Ok(root
        .join("test/fixtures/v1/shasta")
        .join(network)
        .join("proposals")
        .join(format!("proposal_{proposal_id}.fixture.json")))
}

fn repo_relative_path_string(root: &Path, path: &Path) -> Result<String> {
    let relative = path
        .strip_prefix(root)
        .with_context(|| format!("{} must be under repo root", path.display()))?;
    Ok(relative.to_string_lossy().replace('\\', "/"))
}

fn normalize_guest_input(input: &mut GuestInput) {
    if input.taiko.l1_ancestor_headers.is_empty() && input.taiko.l1_header.number != 0 {
        input.taiko.l1_ancestor_headers = vec![input.taiko.l1_header.clone()];
    }
}

fn default_empty_object() -> serde_json::Value {
    serde_json::Value::Object(Default::default())
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("0x{}", hex::encode(Sha256::digest(bytes)))
}

fn normalize_hex(value: &str) -> String {
    let lower = value.trim().to_ascii_lowercase();
    if lower.starts_with("0x") {
        lower
    } else {
        format!("0x{lower}")
    }
}

fn parse_proof_type(raw: &str) -> std::result::Result<ProofType, String> {
    raw.parse()
}

fn print_report(report: &FixtureReport) {
    for result in &report.results {
        if result.passed {
            println!(
                "PASS case={} mode={} proof_type={} commitment={}",
                result.case_id, result.mode, result.proof_type, result.commitment
            );
        } else {
            println!(
                "FAIL case={} mode={} error={}",
                result.case_id,
                result.mode,
                result.error.as_deref().unwrap_or("unknown error")
            );
        }
    }
    println!(
        "fixture: successes={} failures={} elapsed_ms={}",
        report.success_count, report.failure_count, report.elapsed_ms
    );
}

fn elapsed_ms(started_at: Instant) -> u64 {
    u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::B256;
    use raiko2_primitives::ProofType;
    use raiko2_primitives_shasta::{GuestInput, build_proof_carry_data};
    use raiko2_protocol_shasta::libhash::hash_shasta_subproof_input;
    use std::{fs, path::Path};
    use tempfile::TempDir;

    fn repo_root() -> &'static Path {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask lives under repo root")
    }

    fn sample_input_path() -> &'static str {
        "test/guest_inputs/shasta/taiko_hoodi/proposals/proposal_17460.json"
    }

    fn load_sample_input() -> GuestInput {
        let raw = fs::read_to_string(repo_root().join(sample_input_path())).expect("read fixture");
        serde_json::from_str(&raw).expect("parse GuestInput")
    }

    fn sha256_hex(path: &Path) -> String {
        let bytes = fs::read(path).expect("read sha input");
        super::sha256_hex(&bytes)
    }

    fn sample_envelope(proof_type: ProofType) -> FixtureEnvelope {
        let input = load_sample_input();
        let carry = build_proof_carry_data(&input, proof_type).expect("build carry");
        FixtureEnvelope {
            schema: "raiko2-fixture-v1".to_string(),
            case_id: "shasta/taiko_hoodi/proposal_17460".to_string(),
            fork: "shasta".to_string(),
            stage: FixtureStage::Proposal,
            description: None,
            statement: FixtureStatement {
                proof_type,
                public_input_kind: PublicInputKind::ShastaSubproofInput,
            },
            input: FixtureInput {
                kind: FixtureInputKind::ShastaGuestInput,
                path: sample_input_path().to_string(),
                sha256: sha256_hex(&repo_root().join(sample_input_path())),
            },
            expected: FixtureExpected {
                commitment: hash_shasta_subproof_input(&carry),
                opening: FixtureOpening::shasta_proof_carry_data(carry),
            },
            checks: FixtureChecks::default(),
            providers: serde_json::Value::Object(Default::default()),
        }
    }

    #[test]
    fn open_commitment_recomputes_expected_commitment_from_guest_input_and_opening() {
        let envelope = sample_envelope(ProofType::Native);

        let result = check_open_commitment(repo_root(), &envelope).expect("fixture opens");

        assert_eq!(result.case_id, envelope.case_id);
        assert_eq!(result.proof_type, ProofType::Native);
        assert_eq!(result.commitment, envelope.expected.commitment);
        assert!(result.passed);
    }

    #[test]
    fn open_commitment_rejects_inline_opening_that_does_not_match_guest_input() {
        let mut envelope = sample_envelope(ProofType::Native);
        let FixtureOpening::ShastaProofCarryData { proof_carry_data } =
            &mut envelope.expected.opening;
        {
            let carry = proof_carry_data.as_mut();
            carry.transition_input.proposal_id += 1;
            envelope.expected.commitment = hash_shasta_subproof_input(carry);
        }

        let err = check_open_commitment(repo_root(), &envelope)
            .expect_err("stale opening must be rejected");

        assert!(
            err.to_string()
                .contains("fixture opening does not match GuestInput-derived opening")
        );
    }

    #[test]
    fn open_commitment_rejects_envelope_when_check_is_disabled() {
        let mut envelope = sample_envelope(ProofType::Native);
        envelope.checks.open_commitment = false;

        let err = check_open_commitment(repo_root(), &envelope)
            .expect_err("disabled check must be rejected");

        assert!(
            err.to_string()
                .contains("fixture does not enable open-commitment check")
        );
    }

    #[test]
    fn fixture_requires_explicit_proof_type_in_statement() {
        let mut value = serde_json::to_value(sample_envelope(ProofType::Native)).expect("json");
        value
            .get_mut("statement")
            .expect("statement")
            .as_object_mut()
            .expect("statement object")
            .remove("proof_type");

        let err = serde_json::from_value::<FixtureEnvelope>(value)
            .expect_err("missing proof_type must fail");

        assert!(err.to_string().contains("proof_type"));
    }

    #[test]
    fn fixture_defaults_missing_providers_to_empty_object() {
        let mut value = serde_json::to_value(sample_envelope(ProofType::Native)).expect("json");
        value
            .as_object_mut()
            .expect("fixture object")
            .remove("providers");

        let envelope: FixtureEnvelope =
            serde_json::from_value(value).expect("missing providers should default");

        assert_eq!(
            envelope.providers,
            serde_json::Value::Object(Default::default())
        );
    }

    #[test]
    fn check_case_file_writes_json_report_for_open_commitment_mode() {
        let dir = TempDir::new().expect("tempdir");
        let case_path = dir.path().join("proposal_17460.fixture.json");
        let report_path = dir.path().join("report.json");
        fs::write(
            &case_path,
            serde_json::to_string_pretty(&sample_envelope(ProofType::Native)).expect("json"),
        )
        .expect("write fixture");

        let report =
            check_case_file(repo_root(), &case_path, CheckMode::OpenCommitment).expect("check");
        write_report(&report_path, &report).expect("write report");

        let report_json: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&report_path).expect("read report"))
                .expect("parse report");
        assert_eq!(report_json["schema"], "raiko2-fixture-report-v1");
        assert_eq!(
            report_json["results"][0]["case_id"],
            "shasta/taiko_hoodi/proposal_17460"
        );
        assert_eq!(report_json["results"][0]["mode"], "open-commitment");
        assert_eq!(report_json["results"][0]["passed"], true);
    }

    #[test]
    fn public_input_kind_rejects_unknown_values() {
        let err = serde_json::from_str::<PublicInputKind>(r#""shasta_aggregation_input""#)
            .expect_err("phase 1 only supports proposal public input");
        assert!(err.to_string().contains("unknown variant"));
    }

    #[test]
    fn b256_serializes_as_prefixed_hex() {
        let value = serde_json::to_value(FixtureCheckResult {
            case_id: "case".to_string(),
            mode: CheckMode::OpenCommitment,
            passed: true,
            proof_type: ProofType::Native,
            commitment: B256::ZERO,
            error: None,
        })
        .expect("serialize");

        assert_eq!(
            value["commitment"],
            "0x0000000000000000000000000000000000000000000000000000000000000000"
        );
    }

    #[test]
    fn normalize_hex_accepts_uppercase_prefix() {
        assert_eq!(normalize_hex("0XABCDEF"), "0xabcdef");
        assert_eq!(normalize_hex("ABCDEF"), "0xabcdef");
    }

    #[cfg(unix)]
    #[test]
    fn repo_relative_path_rejects_symlink_escape() {
        let root = TempDir::new().expect("root tempdir");
        let outside = TempDir::new().expect("outside tempdir");
        let outside_file = outside.path().join("outside.json");
        fs::write(&outside_file, "{}").expect("write outside file");
        std::os::unix::fs::symlink(&outside_file, root.path().join("input.json"))
            .expect("create symlink");

        let err = resolve_repo_relative_path(root.path(), "input.json")
            .expect_err("symlink escape must be rejected");

        assert!(
            err.to_string()
                .contains("fixture input path must stay within the repo root")
        );
    }

    #[test]
    fn generate_case_file_writes_openable_fixture_envelope() {
        let dir = TempDir::new().expect("tempdir");
        let output = dir.path().join("proposal_17460.fixture.json");

        let generated = generate_case_file(
            repo_root(),
            &FixtureGenerateArgs {
                network: "taiko_hoodi".to_string(),
                proposal: 17460,
                proof_type: ProofType::Native,
                guest_input_root: PathBuf::from("test/guest_inputs/shasta"),
                output: Some(output.clone()),
                case_id: None,
                description: None,
                overwrite: false,
            },
        )
        .expect("generate fixture");

        assert_eq!(generated, output);
        let raw = fs::read_to_string(&output).expect("read generated fixture");
        let envelope: FixtureEnvelope = serde_json::from_str(&raw).expect("parse envelope");
        assert_eq!(envelope.case_id, "shasta/taiko_hoodi/proposal_17460");
        assert_eq!(envelope.statement.proof_type, ProofType::Native);
        assert_eq!(
            envelope.input.path,
            "test/guest_inputs/shasta/taiko_hoodi/proposals/proposal_17460.json"
        );
        check_open_commitment(repo_root(), &envelope).expect("generated fixture opens");
    }

    #[test]
    fn generate_case_file_rejects_existing_output_without_overwrite() {
        let dir = TempDir::new().expect("tempdir");
        let output = dir.path().join("proposal_17460.fixture.json");
        fs::write(&output, "{}").expect("write existing file");

        let err = generate_case_file(
            repo_root(),
            &FixtureGenerateArgs {
                network: "taiko_hoodi".to_string(),
                proposal: 17460,
                proof_type: ProofType::Native,
                guest_input_root: PathBuf::from("test/guest_inputs/shasta"),
                output: Some(output),
                case_id: None,
                description: None,
                overwrite: false,
            },
        )
        .expect_err("existing output must fail");

        assert!(err.to_string().contains("already exists"));
    }

    #[test]
    fn default_fixture_output_path_uses_network_and_proposal() {
        let path = default_fixture_output_path(repo_root(), "taiko_hoodi", 17460)
            .expect("default output path");

        assert_eq!(
            path,
            repo_root()
                .join("test/fixtures/v1/shasta/taiko_hoodi/proposals/proposal_17460.fixture.json")
        );
    }
}
