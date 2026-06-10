use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail, ensure};
use clap::Args;
use serde::{Deserialize, Serialize};
use xtask_build_guest::Backend;

use crate::util;

#[derive(Args)]
pub(crate) struct BenchGuestArgs {
    /// Backend to benchmark (currently supports `sp1` only).
    #[arg(value_enum)]
    pub(crate) backend: Backend,

    /// Path to an existing preflight JSON input.
    ///
    /// If omitted, enough current `preflight` arguments are required to generate the input.
    #[arg(long)]
    pub(crate) input: Option<PathBuf>,

    /// Path to a JSON suite manifest containing existing GuestInput cases.
    #[arg(long)]
    pub(crate) suite: Option<PathBuf>,

    /// Output path for the generated preflight JSON input.
    ///
    /// Only used when `--input` is not provided.
    #[arg(long)]
    pub(crate) output: Option<PathBuf>,

    /// Force re-running preflight even if the output path already exists.
    #[arg(long)]
    pub(crate) force_preflight: bool,

    /// L2 network key in the chain spec list (preflight).
    #[arg(long)]
    pub(crate) network: Option<String>,

    /// L1 network key in the chain spec list (preflight).
    #[arg(long)]
    pub(crate) l1_network: Option<String>,

    /// Upstream RPC URL to fetch block/witness data for preflight.
    #[arg(long)]
    pub(crate) rpc_url: Option<String>,

    /// Optional L1 RPC URL for Shasta anchor linkage.
    #[arg(long)]
    pub(crate) l1_rpc_url: Option<String>,

    /// L2 chain ID for the proof context (preflight).
    #[arg(long)]
    pub(crate) l2_chain_id: Option<u64>,

    /// Proposal ID (L1 event id) to preflight.
    #[arg(long)]
    pub(crate) proposal_id: Option<u64>,

    /// L1 chain ID for the proof context (preflight).
    #[arg(long, default_value_t = 1)]
    pub(crate) l1_chain_id: u64,

    /// L1 block number that contains the Shasta proposal event.
    #[arg(long)]
    pub(crate) l1_inclusion_block_number: Option<u64>,

    /// Last committed anchor block number carried across proposals.
    #[arg(long)]
    pub(crate) last_anchor_block_number: Option<u64>,

    /// L2 block range start (inclusive).
    #[arg(long)]
    pub(crate) l2_start: Option<u64>,

    /// L2 block range end (inclusive).
    #[arg(long)]
    pub(crate) l2_end: Option<u64>,

    /// Proof type to record in the context (preflight).
    #[arg(long, default_value = "sp1")]
    pub(crate) proof_type: String,

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

    /// Execution mode.
    #[arg(long, default_value = "execute")]
    pub(crate) mode: String,

    /// Proof mode when generating proofs (only used when `--mode=prove`).
    #[arg(long, default_value = "plonk")]
    pub(crate) proof_mode: String,

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

#[derive(Clone, Debug, Deserialize, Serialize)]
struct LauncherCycleEntry {
    label: String,
    cycles: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct LauncherCountEntry {
    label: String,
    count: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct LauncherMemoryEntry {
    label: String,
    rss_kb: u64,
    hwm_kb: u64,
}

#[derive(Debug, Deserialize)]
struct BenchSuiteManifest {
    cases: Vec<BenchSuiteCase>,
}

#[derive(Debug, Deserialize)]
struct BenchSuiteCase {
    name: String,
    input: PathBuf,
    #[serde(default)]
    proof_type: Option<String>,
}

#[derive(Debug)]
struct PreparedBenchCase {
    name: String,
    input_path: PathBuf,
    proof_type: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct LauncherReport {
    stage: String,
    mode: String,
    proof_mode: String,
    input: String,
    public_values: String,
    wall_time_ms: u64,
    exit_code: Option<u64>,
    gas: Option<u64>,
    total_instruction_count: Option<u64>,
    total_syscall_count: Option<u64>,
    touched_memory_addresses: Option<u64>,
    cycle_tracker: Vec<LauncherCycleEntry>,
    invocation_tracker: Vec<LauncherCountEntry>,
    opcode_counts: Vec<LauncherCountEntry>,
    syscall_counts: Vec<LauncherCountEntry>,
    memory_snapshots: Vec<LauncherMemoryEntry>,
}

#[derive(Clone, Debug, Serialize)]
struct Stats {
    min: u64,
    max: u64,
    median: u64,
    mean: u64,
}

#[derive(Clone, Debug, Serialize)]
struct BenchSummary {
    wall_time_ms: Option<Stats>,
    prover_gas: Option<Stats>,
    total_instruction_count: Option<Stats>,
    total_syscall_count: Option<Stats>,
    touched_memory_addresses: Option<Stats>,
    total_cycles: Option<Stats>,
    cycles_by_label: BTreeMap<String, Stats>,
}

#[derive(Debug, Serialize)]
struct BenchCaseReport {
    name: String,
    proof_type: String,
    input: String,
    repeat: usize,
    runs: Vec<LauncherReport>,
    summary: BenchSummary,
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
    cases: Vec<BenchCaseReport>,
}

pub(crate) fn run(root: &Path, args: BenchGuestArgs) -> Result<()> {
    ensure!(args.repeat > 0, "--repeat must be > 0");

    if matches!(args.backend, Backend::All) {
        bail!("bench-guest does not support backend=all");
    }
    if !matches!(args.backend, Backend::Sp1) {
        bail!("bench-guest currently supports backend=sp1 only");
    }

    let (input_label, cases) = prepare_bench_cases(root, &args)?;

    let sp1_docker_tag =
        xtask_build_guest::resolve_sp1_docker_tag(root, args.sp1_docker_tag.as_deref());
    let built_guest = !args.skip_build_guest;

    if args.skip_build_guest {
        ensure!(
            root.join("crates/guests/elf/sp1_shasta_proposal.elf")
                .exists(),
            "missing SP1 guest ELF; re-run without `--skip-build-guest` or run `cargo run -r -p xtask -- build-guest sp1 --bench` first"
        );
    } else {
        xtask_build_guest::build(root, args.backend, true, Some(sp1_docker_tag.as_str()))?;
    }

    let launcher_path = build_guest_launcher(root)?;
    let results_dir = util::target_root(root).join("bench/guest");
    fs::create_dir_all(&results_dir).context("create bench results dir")?;

    let run_id = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();

    let mode = args.mode.trim();
    ensure!(!mode.is_empty(), "--mode must not be empty");
    let proof_mode = args.proof_mode.trim();
    ensure!(!proof_mode.is_empty(), "--proof-mode must not be empty");

    let mut all_reports = Vec::new();
    let mut case_reports = Vec::with_capacity(cases.len());
    for (case_index, case) in cases.iter().enumerate() {
        println!(
            "[INFO] Running bench case {}: {} ({})",
            case_index + 1,
            case.name,
            case.input_path.display()
        );
        let measured_reports = run_bench_case(
            &launcher_path,
            &results_dir,
            run_id,
            case_index,
            case,
            mode,
            proof_mode,
            args.warmup,
            args.repeat,
        )?;
        let summary = summarize(&measured_reports);
        print_summary(&summary);
        all_reports.extend(measured_reports.iter().cloned());
        case_reports.push(BenchCaseReport {
            name: case.name.clone(),
            proof_type: case.proof_type.clone(),
            input: case.input_path.display().to_string(),
            repeat: args.repeat,
            runs: measured_reports,
            summary,
        });
    }

    let summary = summarize(&all_reports);

    if let Some(path) = &args.json_out {
        let report = build_bench_report(
            "sp1".to_string(),
            mode.to_string(),
            proof_mode.to_string(),
            input_label,
            built_guest,
            built_guest.then(|| sp1_docker_tag.clone()),
            args.warmup,
            args.repeat,
            all_reports,
            summary,
            case_reports,
        );
        let contents =
            serde_json::to_string_pretty(&report).context("serialize aggregated bench report")?;
        fs::write(path, contents).with_context(|| format!("write {}", path.display()))?;
        println!("[INFO] Wrote aggregated report: {}", path.display());
    } else {
        println!("[INFO] Results directory: {}", results_dir.display());
    }

    Ok(())
}

fn build_bench_report(
    backend: String,
    mode: String,
    proof_mode: String,
    input: String,
    built_guest: bool,
    sp1_docker_tag: Option<String>,
    warmup: usize,
    repeat: usize,
    runs: Vec<LauncherReport>,
    summary: BenchSummary,
    cases: Vec<BenchCaseReport>,
) -> BenchGuestReport {
    BenchGuestReport {
        backend,
        stage: "proposal".to_string(),
        mode,
        proof_mode,
        input,
        built_guest,
        sp1_docker_tag,
        warmup,
        repeat,
        runs,
        summary,
        cases,
    }
}

fn case_name_from_input(input_path: &Path) -> String {
    input_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.trim().is_empty())
        .unwrap_or("input")
        .to_string()
}

fn prepare_bench_cases(
    root: &Path,
    args: &BenchGuestArgs,
) -> Result<(String, Vec<PreparedBenchCase>)> {
    if let Some(suite_path) = &args.suite {
        ensure!(args.input.is_none(), "--suite cannot be used with --input");
        ensure!(
            args.output.is_none(),
            "--suite cannot be used with --output"
        );
        ensure!(
            args.rpc_url.is_none(),
            "--suite cannot be used with --rpc-url"
        );
        ensure!(
            args.l1_rpc_url.is_none(),
            "--suite cannot be used with --l1-rpc-url"
        );
        ensure!(
            args.network.is_none(),
            "--suite cannot be used with --network"
        );
        ensure!(
            args.l1_network.is_none(),
            "--suite cannot be used with --l1-network"
        );
        ensure!(
            args.l2_chain_id.is_none(),
            "--suite cannot be used with --l2-chain-id"
        );
        ensure!(
            args.proposal_id.is_none(),
            "--suite cannot be used with --proposal-id"
        );
        ensure!(
            args.l1_inclusion_block_number.is_none(),
            "--suite cannot be used with --l1-inclusion-block-number"
        );
        ensure!(
            args.last_anchor_block_number.is_none(),
            "--suite cannot be used with --last-anchor-block-number"
        );
        ensure!(
            args.l2_start.is_none(),
            "--suite cannot be used with --l2-start"
        );
        ensure!(
            args.l2_end.is_none(),
            "--suite cannot be used with --l2-end"
        );
        ensure!(
            !args.force_preflight,
            "--suite cannot be used with --force-preflight"
        );

        let suite = read_suite_manifest(suite_path)?;
        let cases = suite
            .cases
            .into_iter()
            .map(|case| PreparedBenchCase {
                name: case.name,
                input_path: case.input,
                proof_type: case.proof_type.unwrap_or_else(|| args.proof_type.clone()),
            })
            .collect();
        return Ok((suite_path.display().to_string(), cases));
    }

    let input_path = prepare_input(root, args)?;
    Ok((
        input_path.display().to_string(),
        vec![PreparedBenchCase {
            name: case_name_from_input(&input_path),
            input_path,
            proof_type: args.proof_type.clone(),
        }],
    ))
}

fn run_bench_case(
    launcher_path: &Path,
    results_dir: &Path,
    run_id: u128,
    case_index: usize,
    case: &PreparedBenchCase,
    mode: &str,
    proof_mode: &str,
    warmup: usize,
    repeat: usize,
) -> Result<Vec<LauncherReport>> {
    let mut measured_reports = Vec::with_capacity(repeat);
    let case_slug = case_result_slug(&case.name, case_index);
    for i in 0..(warmup + repeat) {
        let result_path = results_dir.join(format!(
            "sp1-proposal-{run_id}-{case_slug}-run{}.json",
            i + 1
        ));
        run_guest_launcher(
            launcher_path,
            &case.input_path,
            &case.proof_type,
            mode,
            proof_mode,
            &result_path,
        )?;
        if i >= warmup {
            let report = read_report(&result_path)?;
            measured_reports.push(report);
        }
    }
    Ok(measured_reports)
}

fn case_result_slug(name: &str, index: usize) -> String {
    let slug = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    let slug = slug.trim_matches('_');
    if slug.is_empty() {
        format!("case{}", index + 1)
    } else {
        slug.to_string()
    }
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

    ensure!(
        args.network.is_some() || args.l2_chain_id.is_some(),
        "--network or --l2-chain-id is required when --input is not provided"
    );
    let Some(proposal_id) = args.proposal_id else {
        bail!("--proposal-id is required when --input is not provided");
    };
    let Some(l1_inclusion_block_number) = args.l1_inclusion_block_number else {
        bail!("--l1-inclusion-block-number is required when --input is not provided");
    };
    let Some(last_anchor_block_number) = args.last_anchor_block_number else {
        bail!("--last-anchor-block-number is required when --input is not provided");
    };
    let Some(l2_start) = args.l2_start else {
        bail!("--l2-start is required when --input is not provided");
    };
    let Some(l2_end) = args.l2_end else {
        bail!("--l2-end is required when --input is not provided");
    };

    let target_root = util::target_root(root);
    let network_label = args
        .network
        .clone()
        .or_else(|| args.l2_chain_id.map(|chain_id| format!("l2{chain_id}")))
        .unwrap_or_else(|| "l2".to_string());
    let default_output = target_root.join(format!(
        "bench/input/sp1-{network_label}-proposal{proposal_id}-l2{l2_start}-{l2_end}-l1{l1_inclusion_block_number}-anchor{last_anchor_block_number}.json"
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
    let cmd = build_preflight_command(root, args, &output)?;
    util::run(cmd)?;
    Ok(output)
}

fn build_preflight_command(root: &Path, args: &BenchGuestArgs, output: &Path) -> Result<Command> {
    let Some(proposal_id) = args.proposal_id else {
        bail!("--proposal-id is required when --input is not provided");
    };
    let Some(l1_inclusion_block_number) = args.l1_inclusion_block_number else {
        bail!("--l1-inclusion-block-number is required when --input is not provided");
    };
    let Some(last_anchor_block_number) = args.last_anchor_block_number else {
        bail!("--last-anchor-block-number is required when --input is not provided");
    };
    let Some(l2_start) = args.l2_start else {
        bail!("--l2-start is required when --input is not provided");
    };
    let Some(l2_end) = args.l2_end else {
        bail!("--l2-end is required when --input is not provided");
    };

    let mut cmd = Command::new("cargo");
    cmd.current_dir(root);
    cmd.arg("run")
        .arg("--locked")
        .arg("--bin")
        .arg("preflight")
        .arg("--");

    if let Some(network) = &args.network {
        cmd.arg(format!("--network={network}"));
    }
    if let Some(l1_network) = &args.l1_network {
        cmd.arg(format!("--l1-network={l1_network}"));
    }
    if let Some(rpc_url) = &args.rpc_url {
        cmd.arg(format!("--rpc-url={rpc_url}"));
    }
    if let Some(l1_rpc_url) = &args.l1_rpc_url {
        cmd.arg(format!("--l1-rpc-url={l1_rpc_url}"));
    }
    if let Some(l2_chain_id) = args.l2_chain_id {
        cmd.arg(format!("--l2-chain-id={l2_chain_id}"));
    }
    cmd.arg(format!("--l1-chain-id={}", args.l1_chain_id))
        .arg(format!("--proposal-id={proposal_id}"))
        .arg(format!(
            "--l1-inclusion-block-number={l1_inclusion_block_number}"
        ))
        .arg(format!(
            "--last-anchor-block-number={last_anchor_block_number}"
        ))
        .arg(format!("--l2-start={l2_start}"))
        .arg(format!("--l2-end={l2_end}"))
        .arg(format!("--proof-type={}", args.proof_type))
        .arg("-o")
        .arg(output);

    if args.validate {
        cmd.arg("--validate");
    }
    if args.pretty {
        cmd.arg("--pretty");
    }

    if let Some(prover) = &args.prover {
        cmd.arg(format!("--prover={prover}"));
    }
    if let Some(graffiti) = &args.graffiti {
        cmd.arg(format!("--graffiti={graffiti}"));
    }

    Ok(cmd)
}

fn build_guest_launcher(root: &Path) -> Result<PathBuf> {
    println!("[INFO] Building guest-launcher (release, SP1 profiling enabled)...");
    let mut cmd = Command::new("cargo");
    cmd.current_dir(root);
    cmd.arg("build")
        .arg("--locked")
        .arg("--release")
        .arg("--bin")
        .arg("guest-launcher")
        .arg("--features")
        .arg("sp1-sdk/profiling");
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
    proof_type: &str,
    mode: &str,
    proof_mode: &str,
    json_out: &Path,
) -> Result<()> {
    println!("[INFO] Running guest-launcher ({mode})");
    let mut cmd = Command::new(launcher);
    cmd.arg("--input")
        .arg(input)
        .arg("--proof-type")
        .arg(proof_type)
        .arg("--mode")
        .arg(mode)
        .arg("--proof-mode")
        .arg(proof_mode)
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

fn read_suite_manifest(path: &Path) -> Result<BenchSuiteManifest> {
    let contents = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let manifest: BenchSuiteManifest =
        serde_json::from_str(&contents).context("parse bench suite manifest")?;
    ensure!(
        !manifest.cases.is_empty(),
        "bench suite must contain at least one case"
    );
    for (index, case) in manifest.cases.iter().enumerate() {
        ensure!(
            !case.name.trim().is_empty(),
            "bench suite case {index} name must not be empty"
        );
        ensure!(
            case.input.exists(),
            "bench suite case {} input does not exist: {}",
            case.name,
            case.input.display()
        );
        if let Some(proof_type) = case.proof_type.as_deref() {
            ensure!(
                !proof_type.trim().is_empty(),
                "bench suite case {} proof_type must not be empty",
                case.name
            );
        }
    }
    Ok(manifest)
}

fn summarize(reports: &[LauncherReport]) -> BenchSummary {
    let wall_times: Vec<u64> = reports.iter().map(|r| r.wall_time_ms).collect();
    let prover_gas: Vec<u64> = reports.iter().filter_map(|r| r.gas).collect();
    let total_instruction_counts: Vec<u64> = reports
        .iter()
        .filter_map(|r| r.total_instruction_count)
        .collect();
    let total_syscall_counts: Vec<u64> = reports
        .iter()
        .filter_map(|r| r.total_syscall_count)
        .collect();
    let touched_memory_addresses: Vec<u64> = reports
        .iter()
        .filter_map(|r| r.touched_memory_addresses)
        .collect();
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
        prover_gas: stats(&prover_gas),
        total_instruction_count: stats(&total_instruction_counts),
        total_syscall_count: stats(&total_syscall_counts),
        touched_memory_addresses: stats(&touched_memory_addresses),
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
    if let Some(stats) = &summary.prover_gas {
        println!(
            "prover_gas: min={} median={} mean={} max={}",
            stats.min, stats.median, stats.mean, stats.max
        );
    }
    if let Some(stats) = &summary.total_instruction_count {
        println!(
            "total_instruction_count: min={} median={} mean={} max={}",
            stats.min, stats.median, stats.mean, stats.max
        );
    }
    if let Some(stats) = &summary.total_syscall_count {
        println!(
            "total_syscall_count: min={} median={} mean={} max={}",
            stats.min, stats.median, stats.mean, stats.max
        );
    }
    if let Some(stats) = &summary.touched_memory_addresses {
        println!(
            "touched_memory_addresses: min={} median={} mean={} max={}",
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

#[cfg(test)]
mod tests {
    use super::*;

    fn launcher_report_with_metrics(
        wall_time_ms: u64,
        gas: Option<u64>,
        total_instruction_count: Option<u64>,
        total_syscall_count: Option<u64>,
        touched_memory_addresses: Option<u64>,
    ) -> LauncherReport {
        LauncherReport {
            stage: "proposal".to_string(),
            mode: "execute".to_string(),
            proof_mode: "compressed".to_string(),
            input: "input.json".to_string(),
            public_values: "0x1234".to_string(),
            wall_time_ms,
            exit_code: Some(0),
            gas,
            total_instruction_count,
            total_syscall_count,
            touched_memory_addresses,
            cycle_tracker: Vec::new(),
            invocation_tracker: Vec::new(),
            opcode_counts: Vec::new(),
            syscall_counts: Vec::new(),
            memory_snapshots: Vec::new(),
        }
    }

    #[test]
    fn launcher_report_preserves_sp1_execute_metadata() {
        let raw = r#"{
          "stage": "proposal",
          "mode": "execute",
          "proof_mode": "compressed",
          "input": "input.json",
          "public_values": "0x1234",
          "wall_time_ms": 42,
          "exit_code": 0,
          "gas": 123456,
          "total_instruction_count": 200,
          "total_syscall_count": 3,
          "touched_memory_addresses": 99,
          "cycle_tracker": [{"label": "block", "cycles": 10}],
          "invocation_tracker": [{"label": "block", "count": 1}],
          "opcode_counts": [{"label": "ADD", "count": 2}],
          "syscall_counts": [{"label": "COMMIT", "count": 1}],
          "memory_snapshots": [{"label": "proposal:start", "rss_kb": 10, "hwm_kb": 12}]
        }"#;

        let report: LauncherReport = serde_json::from_str(raw).unwrap();

        assert_eq!(report.gas, Some(123456));
        assert_eq!(report.exit_code, Some(0));
        assert_eq!(report.total_instruction_count, Some(200));
        assert_eq!(report.total_syscall_count, Some(3));
        assert_eq!(report.touched_memory_addresses, Some(99));
        assert_eq!(report.invocation_tracker[0].label, "block");
        assert_eq!(report.opcode_counts[0].label, "ADD");
        assert_eq!(report.syscall_counts[0].label, "COMMIT");
        assert_eq!(report.memory_snapshots[0].label, "proposal:start");
    }

    #[test]
    fn bench_summary_includes_prover_gas_and_execution_counters() {
        let reports = vec![
            launcher_report_with_metrics(10, Some(100), Some(1_000), Some(3), Some(50)),
            launcher_report_with_metrics(20, Some(200), Some(3_000), Some(5), Some(70)),
        ];

        let summary = summarize(&reports);

        assert_eq!(summary.prover_gas.unwrap().median, 200);
        assert_eq!(summary.total_instruction_count.unwrap().mean, 2_000);
        assert_eq!(summary.total_syscall_count.unwrap().max, 5);
        assert_eq!(summary.touched_memory_addresses.unwrap().min, 50);
    }

    #[test]
    fn bench_suite_manifest_parses_guest_input_cases() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("input.json");
        fs::write(&input, "{}").unwrap();
        let suite_path = dir.path().join("suite.json");
        fs::write(
            &suite_path,
            format!(
                r#"{{
                  "cases": [
                    {{
                      "name": "mainnet-proposal-2222",
                      "input": "{}",
                      "proof_type": "sp1"
                    }}
                  ]
                }}"#,
                input.display()
            ),
        )
        .unwrap();

        let suite = read_suite_manifest(&suite_path).unwrap();

        assert_eq!(suite.cases.len(), 1);
        assert_eq!(suite.cases[0].name, "mainnet-proposal-2222");
        assert_eq!(suite.cases[0].input, input);
        assert_eq!(suite.cases[0].proof_type.as_deref(), Some("sp1"));
    }

    #[test]
    fn preflight_command_uses_current_shasta_tuple_args() {
        let root = Path::new("/repo");
        let output = Path::new("/tmp/input.json");
        let args = BenchGuestArgs {
            backend: Backend::Sp1,
            input: None,
            suite: None,
            output: Some(output.to_path_buf()),
            force_preflight: true,
            network: Some("taiko_dev".to_string()),
            l1_network: Some("taiko_dev_l1".to_string()),
            rpc_url: Some("https://rpc.internal.taiko.xyz".to_string()),
            l1_rpc_url: Some("https://l1rpc.internal.taiko.xyz".to_string()),
            l2_chain_id: Some(167001),
            l1_chain_id: 32382,
            proposal_id: Some(730),
            l1_inclusion_block_number: Some(20802),
            last_anchor_block_number: Some(20734),
            l2_start: Some(239062),
            l2_end: Some(239445),
            proof_type: "sp1".to_string(),
            prover: None,
            graffiti: None,
            validate: false,
            pretty: true,
            skip_build_guest: true,
            sp1_docker_tag: None,
            mode: "execute".to_string(),
            proof_mode: "plonk".to_string(),
            warmup: 0,
            repeat: 1,
            json_out: None,
        };

        let cmd = build_preflight_command(root, &args, output).unwrap();
        let args = cmd
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert!(args.contains(&"--network=taiko_dev".to_string()));
        assert!(args.contains(&"--l1-network=taiko_dev_l1".to_string()));
        assert!(args.contains(&"--rpc-url=https://rpc.internal.taiko.xyz".to_string()));
        assert!(args.contains(&"--l1-rpc-url=https://l1rpc.internal.taiko.xyz".to_string()));
        assert!(args.contains(&"--proposal-id=730".to_string()));
        assert!(args.contains(&"--l1-inclusion-block-number=20802".to_string()));
        assert!(args.contains(&"--last-anchor-block-number=20734".to_string()));
        assert!(args.contains(&"--l2-start=239062".to_string()));
        assert!(args.contains(&"--l2-end=239445".to_string()));
        assert!(!args.iter().any(|arg| arg.contains("debug-witness")));
    }

    #[test]
    fn bench_suite_report_groups_case_results() {
        let runs = vec![launcher_report_with_metrics(
            10,
            Some(123_456),
            Some(1_000),
            Some(3),
            Some(50),
        )];
        let case = BenchCaseReport {
            name: "mainnet-proposal-2222".to_string(),
            proof_type: "sp1".to_string(),
            input: "input.json".to_string(),
            repeat: 1,
            runs,
            summary: summarize(&[launcher_report_with_metrics(
                10,
                Some(123_456),
                Some(1_000),
                Some(3),
                Some(50),
            )]),
        };
        let report = build_bench_report(
            "sp1".to_string(),
            "execute".to_string(),
            "compressed".to_string(),
            "suite.json".to_string(),
            true,
            None,
            0,
            1,
            Vec::new(),
            summarize(&[]),
            vec![case],
        );

        let value = serde_json::to_value(report).unwrap();

        assert_eq!(value["cases"][0]["name"], "mainnet-proposal-2222");
        assert_eq!(value["cases"][0]["runs"][0]["gas"], 123_456);
        assert_eq!(
            value["cases"][0]["summary"]["prover_gas"]["median"],
            123_456
        );
    }
}
