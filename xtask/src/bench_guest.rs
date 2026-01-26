use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail, ensure};
use clap::{Args, ValueEnum};
use serde::{Deserialize, Serialize};

use crate::Backend;
use crate::build_guest;
use crate::util;

#[derive(Args)]
pub(crate) struct BenchGuestArgs {
    /// Backend to benchmark (currently supports `sp1` only).
    #[arg(value_enum)]
    pub(crate) backend: Backend,

    /// Path to an existing preflight JSON input.
    ///
    /// If omitted, `--rpc-url`, `--l2-chain-id`, and `--proposal-id` are required and `preflight`
    /// will be run to generate the input.
    #[arg(long)]
    pub(crate) input: Option<PathBuf>,

    /// Output path for the generated preflight JSON input.
    ///
    /// Only used when `--input` is not provided.
    #[arg(long)]
    pub(crate) output: Option<PathBuf>,

    /// Force re-running preflight even if the output path already exists.
    #[arg(long)]
    pub(crate) force_preflight: bool,

    /// Upstream RPC URL to fetch block/witness data for preflight.
    #[arg(long)]
    pub(crate) rpc_url: Option<String>,

    /// L2 chain ID for the proof context (preflight).
    #[arg(long)]
    pub(crate) l2_chain_id: Option<u64>,

    /// Proposal ID (block number) to preflight.
    #[arg(long)]
    pub(crate) proposal_id: Option<u64>,

    /// L1 chain ID for the proof context (preflight).
    #[arg(long, default_value_t = 1)]
    pub(crate) l1_chain_id: u64,

    /// Proof type to record in the context (preflight).
    #[arg(long, default_value = "sp1")]
    pub(crate) proof_type: String,

    /// Whether the RPC supports `debug_executionWitness` (preflight).
    #[arg(long)]
    pub(crate) debug_witness: bool,

    /// Optional prover address to embed in the manifest (preflight).
    #[arg(long)]
    pub(crate) prover: Option<String>,

    /// Optional graffiti to embed in the manifest (preflight).
    #[arg(long)]
    pub(crate) graffiti: Option<String>,

    /// Validate the guest input after preflight.
    #[arg(long)]
    pub(crate) validate: bool,

    /// Pretty-print the generated JSON input.
    #[arg(long)]
    pub(crate) pretty: bool,

    /// Skip rebuilding guest ELFs and reuse the existing ones under `crates/guests/elf`.
    ///
    /// By default, `bench-guest` rebuilds SP1 guest ELFs (docker) so the embedded program matches
    /// the current code and input schema.
    #[arg(long)]
    pub(crate) skip_build_guest: bool,

    /// Override SP1 docker tag for guest builds (equivalent to setting `SP1_DOCKER_TAG`).
    #[arg(long)]
    pub(crate) sp1_docker_tag: Option<String>,

    /// Proof stage to run.
    #[arg(long, value_enum, default_value = "proposal")]
    pub(crate) stage: BenchStage,

    /// Execution mode.
    #[arg(long, value_enum, default_value = "execute")]
    pub(crate) mode: BenchMode,

    /// Proof mode when generating proofs (only used when `--mode=prove`).
    #[arg(long, value_enum, default_value = "plonk")]
    pub(crate) proof_mode: BenchProofMode,

    /// Warmup runs (not included in summary).
    #[arg(long, default_value_t = 0)]
    pub(crate) warmup: usize,

    /// Measured runs.
    #[arg(long, default_value_t = 1)]
    pub(crate) repeat: usize,

    /// Optional path to write an aggregated JSON report.
    #[arg(long)]
    pub(crate) json_out: Option<PathBuf>,
}

#[derive(ValueEnum, Clone, Copy, Debug)]
pub(crate) enum BenchStage {
    Proposal,
    Aggregation,
}

#[derive(ValueEnum, Clone, Copy, Debug)]
pub(crate) enum BenchMode {
    Execute,
    Prove,
}

#[derive(ValueEnum, Clone, Copy, Debug)]
pub(crate) enum BenchProofMode {
    Core,
    Compressed,
    Plonk,
}

impl BenchStage {
    const fn as_str(self) -> &'static str {
        match self {
            BenchStage::Proposal => "proposal",
            BenchStage::Aggregation => "aggregation",
        }
    }
}

impl BenchMode {
    const fn as_str(self) -> &'static str {
        match self {
            BenchMode::Execute => "execute",
            BenchMode::Prove => "prove",
        }
    }
}

impl BenchProofMode {
    const fn as_str(self) -> &'static str {
        match self {
            BenchProofMode::Core => "core",
            BenchProofMode::Compressed => "compressed",
            BenchProofMode::Plonk => "plonk",
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct LauncherCycleEntry {
    label: String,
    cycles: u64,
}

#[derive(Debug, Deserialize, Serialize)]
struct LauncherReport {
    stage: String,
    mode: String,
    proof_mode: String,
    input: String,
    public_values: String,
    wall_time_ms: u64,
    cycle_tracker: Vec<LauncherCycleEntry>,
}

#[derive(Debug, Serialize)]
struct Stats {
    min: u64,
    max: u64,
    median: u64,
    mean: u64,
}

#[derive(Debug, Serialize)]
struct BenchSummary {
    wall_time_ms: Option<Stats>,
    total_cycles: Option<Stats>,
    cycles_by_label: BTreeMap<String, Stats>,
}

#[derive(Debug, Serialize)]
struct BenchGuestReport {
    backend: String,
    stage: String,
    mode: String,
    proof_mode: String,
    input: String,
    built_guest: bool,
    sp1_docker_tag: Option<String>,
    warmup: usize,
    repeat: usize,
    runs: Vec<LauncherReport>,
    summary: BenchSummary,
}

pub(crate) fn run(root: &Path, args: BenchGuestArgs) -> Result<()> {
    ensure!(args.repeat > 0, "--repeat must be > 0");

    if matches!(args.backend, Backend::All) {
        bail!("bench-guest does not support backend=all");
    }
    if !matches!(args.backend, Backend::Sp1) {
        bail!("bench-guest currently supports backend=sp1 only");
    }
    if matches!(args.stage, BenchStage::Aggregation) {
        bail!("bench-guest currently supports stage=proposal only");
    }

    let input_path = prepare_input(root, &args)?;

    let sp1_docker_tag = build_guest::resolve_sp1_docker_tag(root, args.sp1_docker_tag.as_deref());
    let built_guest = !args.skip_build_guest;

    if args.skip_build_guest {
        ensure!(
            root.join("crates/guests/elf/sp1_shasta_proposal.elf")
                .exists(),
            "missing SP1 guest ELF; re-run without `--skip-build-guest` or run `cargo run -r -p xtask -- build-guest sp1 --bench` first"
        );
    } else {
        build_guest::build(
            root,
            args.backend,
            true,
            Some(sp1_docker_tag.as_str()),
            false,
        )?;
    }

    let launcher_path = build_guest_launcher(root)?;
    let results_dir = util::target_root(root).join("bench/guest");
    fs::create_dir_all(&results_dir).context("create bench results dir")?;

    let run_id = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();

    let mut measured_reports = Vec::with_capacity(args.repeat);
    for i in 0..(args.warmup + args.repeat) {
        let result_path = results_dir.join(format!(
            "sp1-{}-{}-{}-{}-run{}.json",
            args.stage.as_str(),
            args.mode.as_str(),
            args.proof_mode.as_str(),
            run_id,
            i + 1
        ));
        run_guest_launcher(
            &launcher_path,
            &input_path,
            args.stage,
            args.mode,
            args.proof_mode,
            &result_path,
        )?;
        if i >= args.warmup {
            let report = read_report(&result_path)?;
            measured_reports.push(report);
        }
    }

    let summary = summarize(&measured_reports);
    print_summary(&summary);

    if let Some(path) = &args.json_out {
        let report = BenchGuestReport {
            backend: "sp1".to_string(),
            stage: args.stage.as_str().to_string(),
            mode: args.mode.as_str().to_string(),
            proof_mode: args.proof_mode.as_str().to_string(),
            input: input_path.display().to_string(),
            built_guest,
            sp1_docker_tag: built_guest.then(|| sp1_docker_tag.clone()),
            warmup: args.warmup,
            repeat: args.repeat,
            runs: measured_reports,
            summary,
        };
        let contents =
            serde_json::to_string_pretty(&report).context("serialize aggregated bench report")?;
        fs::write(path, contents).with_context(|| format!("write {}", path.display()))?;
        println!("[INFO] Wrote aggregated report: {}", path.display());
    } else {
        println!("[INFO] Results directory: {}", results_dir.display());
    }

    Ok(())
}

fn prepare_input(root: &Path, args: &BenchGuestArgs) -> Result<PathBuf> {
    if let Some(path) = &args.input {
        ensure!(
            path.exists(),
            "input file does not exist: {}",
            path.display()
        );
        return Ok(path.clone());
    }

    let Some(rpc_url) = &args.rpc_url else {
        bail!("--rpc-url is required when --input is not provided");
    };
    let Some(l2_chain_id) = args.l2_chain_id else {
        bail!("--l2-chain-id is required when --input is not provided");
    };
    let Some(proposal_id) = args.proposal_id else {
        bail!("--proposal-id is required when --input is not provided");
    };

    let target_root = util::target_root(root);
    let default_output = target_root.join(format!(
        "bench/input/sp1-l2{l2_chain_id}-proposal{proposal_id}.json"
    ));
    let output = args.output.clone().unwrap_or(default_output);

    if output.exists() && !args.force_preflight {
        println!("[INFO] Reusing existing input: {}", output.display());
        return Ok(output);
    }

    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent).context("create bench input dir")?;
    }

    println!(
        "[INFO] Running preflight to generate input: {}",
        output.display()
    );
    let mut cmd = Command::new("cargo");
    cmd.current_dir(root);
    cmd.arg("run")
        .arg("--locked")
        .arg("--bin")
        .arg("preflight")
        .arg("--")
        .arg(format!("--rpc-url={rpc_url}"))
        .arg(format!("--l2-chain-id={l2_chain_id}"))
        .arg(format!("--l1-chain-id={}", args.l1_chain_id))
        .arg(format!("--proposal-id={proposal_id}"))
        .arg(format!("--proof-type={}", args.proof_type))
        .arg(format!("--debug-witness={}", args.debug_witness))
        .arg(format!("--validate={}", args.validate))
        .arg("-o")
        .arg(&output)
        .arg(format!("--pretty={}", args.pretty));

    if let Some(prover) = &args.prover {
        cmd.arg(format!("--prover={prover}"));
    }
    if let Some(graffiti) = &args.graffiti {
        cmd.arg(format!("--graffiti={graffiti}"));
    }

    util::run(cmd)?;
    Ok(output)
}

fn build_guest_launcher(root: &Path) -> Result<PathBuf> {
    println!("[INFO] Building guest-launcher (release)...");
    let mut cmd = Command::new("cargo");
    cmd.current_dir(root);
    cmd.arg("build")
        .arg("--locked")
        .arg("--release")
        .arg("--bin")
        .arg("guest-launcher");
    util::run(cmd)?;

    let bin_name = if cfg!(windows) {
        "guest-launcher.exe"
    } else {
        "guest-launcher"
    };
    Ok(util::target_root(root).join("release").join(bin_name))
}

fn run_guest_launcher(
    launcher: &Path,
    input: &Path,
    stage: BenchStage,
    mode: BenchMode,
    proof_mode: BenchProofMode,
    json_out: &Path,
) -> Result<()> {
    println!(
        "[INFO] Running guest-launcher ({} {})",
        stage.as_str(),
        mode.as_str()
    );
    let mut cmd = Command::new(launcher);
    cmd.arg("--input")
        .arg(input)
        .arg("--stage")
        .arg(stage.as_str())
        .arg("--mode")
        .arg(mode.as_str())
        .arg("--proof-mode")
        .arg(proof_mode.as_str())
        .arg("--json-out")
        .arg(json_out);
    let status = cmd
        .status()
        .with_context(|| format!("failed to run {cmd:?}"))?;
    if !status.success() {
        bail!(
            "guest-launcher failed ({status}). If this is a GuestInput deserialization panic, \
             rebuild SP1 guest ELFs by re-running without `--skip-build-guest` (or `cargo run -r -p xtask -- build-guest sp1 --bench`). Command: {cmd:?}"
        );
    }
    Ok(())
}

fn read_report(path: &Path) -> Result<LauncherReport> {
    let contents = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let report: LauncherReport =
        serde_json::from_str(&contents).context("parse guest-launcher JSON report")?;
    Ok(report)
}

fn summarize(reports: &[LauncherReport]) -> BenchSummary {
    let wall_times: Vec<u64> = reports.iter().map(|r| r.wall_time_ms).collect();
    let total_cycles: Vec<u64> = reports.iter().map(total_cycles).collect();

    let mut by_label: BTreeMap<String, Vec<u64>> = BTreeMap::new();
    for report in reports {
        for entry in &report.cycle_tracker {
            by_label
                .entry(entry.label.clone())
                .or_default()
                .push(entry.cycles);
        }
    }

    BenchSummary {
        wall_time_ms: stats(&wall_times),
        total_cycles: stats(&total_cycles),
        cycles_by_label: by_label
            .into_iter()
            .filter_map(|(label, values)| stats(&values).map(|s| (label, s)))
            .collect(),
    }
}

fn total_cycles(report: &LauncherReport) -> u64 {
    report.cycle_tracker.iter().map(|e| e.cycles).sum::<u64>()
}

fn stats(values: &[u64]) -> Option<Stats> {
    if values.is_empty() {
        return None;
    }

    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let min = *sorted.first().unwrap_or(&0);
    let max = *sorted.last().unwrap_or(&0);
    let median = sorted[sorted.len() / 2];
    let sum = sorted.iter().copied().map(u128::from).sum::<u128>();
    let count = u64::try_from(sorted.len()).unwrap_or(u64::MAX);
    let mean = sum / u128::from(count);
    let mean = u64::try_from(mean).unwrap_or(u64::MAX);

    Some(Stats {
        min,
        max,
        median,
        mean,
    })
}

fn print_summary(summary: &BenchSummary) {
    println!();
    println!("=== guest bench summary ===");

    if let Some(stats) = &summary.wall_time_ms {
        println!(
            "wall_time_ms: min={} median={} mean={} max={}",
            stats.min, stats.median, stats.mean, stats.max
        );
    }
    if let Some(stats) = &summary.total_cycles {
        println!(
            "total_cycles: min={} median={} mean={} max={}",
            stats.min, stats.median, stats.mean, stats.max
        );
    }

    if summary.cycles_by_label.is_empty() {
        println!("cycle_tracker: <empty>");
        println!("note: build SP1 guest with `--bench` to enable cycle tracking");
        return;
    }

    println!("cycle_tracker (median cycles):");
    for (label, stats) in &summary.cycles_by_label {
        println!("  {label}: {}", stats.median);
    }
}
