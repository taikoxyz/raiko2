use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use alloy_primitives::{Address, B256, hex};
use anyhow::{Context, Result, bail};
use clap::{Parser, ValueEnum};
use raiko2_pipeline::forks::shasta::{load_risc0_shasta_backend, load_sp1_shasta_backend};
use raiko2_pipeline::{NativeBackend, ProofStage, ProverBackend};
use raiko2_primitives::{OpcodeLabInput, PrecompileLabInput, Proof, ProofType as RaikoProofType};
use raiko2_primitives_shasta::build_proof_carry_data;
use raiko2_primitives_shasta::decode_proof_carry_data;
use raiko2_primitives_shasta::encode_proof_carry_data;
use raiko2_primitives_shasta::instance::words_to_bytes_be;
use raiko2_primitives_shasta::{GuestInput, ShastaZkAggregationGuestInput};
use raiko2_protocol_shasta::libhash::hash_shasta_subproof_input;
use raiko2_protocol_shasta::shasta::ProofCarryData;
use raiko2_prover::Prover;
use raiko2_prover::native::NativeProver;
use raiko2_prover::sp1::{
    ProverMode as Sp1ProverMode, Sp1Config, Sp1FulfillmentStrategy, Sp1NetworkMode,
    encode_sp1_aggregation_proof_payload, encode_sp1_proposal_proof_payload,
    load_sp1_subproof_for_aggregation, sp1_image_id_words_from_uuid, sp1_vk_uuid,
};
use serde::{Deserialize, Serialize};
use sp1_sdk::utils::setup_logger;
use sp1_sdk::{
    ExecutionReport, NetworkProver, ProveRequest as _, Prover as _, ProvingKey as _, SP1Proof,
    SP1ProofMode, SP1ProofWithPublicValues, SP1ProvingKey, SP1Stdin, SP1VerifyingKey,
    blocking::{ProveRequest as _, Prover as BlockingProver, ProverClient as BlockingProverClient},
    network::{
        NetworkMode as Sp1SdkNetworkMode, get_default_rpc_url_for_mode, signer::NetworkSigner,
    },
};

#[derive(Parser)]
#[command(name = "guest-launcher")]
#[command(about = "Run SP1 guest programs locally with JSON inputs", long_about = None)]
struct Args {
    /// Path to the input JSON file.
    #[arg(long)]
    input: Option<PathBuf>,
    /// JSON file containing a list of lab input paths.
    #[arg(long)]
    input_list: Option<PathBuf>,
    /// Guest execution stage.
    #[arg(long, value_enum, default_value = "proposal")]
    stage: Stage,
    /// Explicit guest ELF path. Overrides the built-in proposal ELF and is required for labs.
    #[arg(long)]
    elf: Option<PathBuf>,
    /// Proof files to aggregate.
    #[arg(long, num_args = 1..)]
    aggregate: Vec<PathBuf>,
    /// Execution mode (execute for simulation, prove for proof generation).
    #[arg(long, value_enum, default_value = "execute")]
    mode: Mode,
    /// Proof mode when generating proofs. Defaults to compressed for proposals and plonk for aggregation.
    #[arg(long, value_enum)]
    proof_mode: Option<ProofMode>,
    /// Proof backend to use.
    #[arg(long, value_enum, default_value = "native")]
    proof_type: ProofType,
    /// Path to write proof JSON output.
    #[arg(long)]
    output: Option<PathBuf>,

    /// Optional path to write a JSON benchmark report.
    #[arg(long)]
    json_out: Option<PathBuf>,
    /// Optional path to write JSONL benchmark reports for batch runs.
    #[arg(long)]
    jsonl_out: Option<PathBuf>,
    /// Override the SP1 prover mode. Defaults to `local` for execute and `network` for prove.
    #[arg(long, value_enum)]
    sp1_prover: Option<CliSp1ProverMode>,
    /// Succinct network mode for SP1 remote proving.
    #[arg(long, value_enum, default_value = "reserved")]
    sp1_network_mode: CliSp1NetworkMode,
    /// Succinct fulfillment strategy for SP1 remote proving.
    #[arg(long, value_enum, default_value = "reserved")]
    sp1_fulfillment_strategy: CliSp1FulfillmentStrategy,
    /// Skip local simulation before submitting an SP1 network proof.
    #[arg(long, default_value_t = true)]
    sp1_skip_simulation: bool,
    /// Cycle limit for SP1 proving.
    #[arg(long, default_value_t = 1_000_000_000_000)]
    sp1_cycle_limit: u64,
    /// Timeout in seconds when waiting for an SP1 network proof.
    #[arg(long, default_value_t = 3_600)]
    sp1_timeout_secs: u64,
    /// RISC0 segment limit for local execute dry-runs.
    #[arg(long, default_value_t = 20)]
    risc0_execution_po2: u32,
}

#[derive(Clone, Copy, Debug, ValueEnum, PartialEq, Eq)]
enum Mode {
    Execute,
    Prove,
}

#[derive(Clone, Copy, Debug, ValueEnum, PartialEq, Eq)]
enum ProofType {
    Native,
    Risc0,
    Sp1,
}

#[derive(Clone, Copy, Debug, ValueEnum, PartialEq, Eq)]
enum Stage {
    Proposal,
    #[value(name = "opcode-lab")]
    OpcodeLab,
    #[value(name = "revm-opcode-lab")]
    RevmOpcodeLab,
    #[value(name = "precompile-lab")]
    PrecompileLab,
}

#[derive(Clone, Copy, Debug, ValueEnum, PartialEq, Eq)]
enum ProofMode {
    Core,
    Compressed,
    Plonk,
}

#[derive(Clone, Copy, Debug, ValueEnum, PartialEq, Eq)]
enum CliSp1ProverMode {
    Mock,
    Local,
    Network,
}

#[derive(Clone, Copy, Debug, ValueEnum, PartialEq, Eq)]
enum CliSp1NetworkMode {
    Reserved,
    Mainnet,
}

#[derive(Clone, Copy, Debug, ValueEnum, PartialEq, Eq)]
enum CliSp1FulfillmentStrategy {
    Reserved,
    Hosted,
    Auction,
}

#[derive(Debug, Serialize)]
struct BenchCycleEntry {
    label: String,
    cycles: u64,
}

#[derive(Clone, Debug, Serialize)]
struct BenchCountEntry {
    label: String,
    count: u64,
}

#[derive(Debug, Serialize)]
struct BenchMemoryEntry {
    label: String,
    rss_kb: u64,
    hwm_kb: u64,
}

#[derive(Debug, Serialize)]
struct BenchReport {
    stage: &'static str,
    mode: &'static str,
    proof_mode: &'static str,
    input: String,
    public_values: String,
    wall_time_ms: u64,
    primary_workload_metric: Option<BenchCountEntry>,
    workload_metrics: Vec<BenchCountEntry>,
    exit_code: Option<u64>,
    gas: Option<u64>,
    total_instruction_count: Option<u64>,
    total_syscall_count: Option<u64>,
    touched_memory_addresses: Option<u64>,
    risc0_user_cycles: Option<u64>,
    risc0_padded_cycles: Option<u64>,
    risc0_segment_count: Option<u64>,
    risc0_po2_counts: Vec<BenchCountEntry>,
    cycle_tracker: Vec<BenchCycleEntry>,
    invocation_tracker: Vec<BenchCountEntry>,
    opcode_counts: Vec<BenchCountEntry>,
    syscall_counts: Vec<BenchCountEntry>,
    memory_snapshots: Vec<BenchMemoryEntry>,
}

impl BenchReport {
    fn new(
        stage: &'static str,
        mode: &'static str,
        proof_mode: &'static str,
        input: String,
    ) -> Self {
        Self {
            stage,
            mode,
            proof_mode,
            input,
            public_values: String::new(),
            wall_time_ms: 0,
            primary_workload_metric: None,
            workload_metrics: Vec::new(),
            exit_code: None,
            gas: None,
            total_instruction_count: None,
            total_syscall_count: None,
            touched_memory_addresses: None,
            risc0_user_cycles: None,
            risc0_padded_cycles: None,
            risc0_segment_count: None,
            risc0_po2_counts: Vec::new(),
            cycle_tracker: Vec::new(),
            invocation_tracker: Vec::new(),
            opcode_counts: Vec::new(),
            syscall_counts: Vec::new(),
            memory_snapshots: Vec::new(),
        }
    }

    fn push_workload_metric(&mut self, label: &'static str, count: u64) {
        self.workload_metrics.push(BenchCountEntry {
            label: label.to_string(),
            count,
        });
    }

    fn set_primary_workload_metric(&mut self, label: &'static str, count: u64) {
        let entry = BenchCountEntry {
            label: label.to_string(),
            count,
        };
        self.primary_workload_metric = Some(entry);
        self.push_workload_metric(label, count);
    }
}

impl Mode {
    const fn as_str(self) -> &'static str {
        match self {
            Mode::Execute => "execute",
            Mode::Prove => "prove",
        }
    }
}

impl ProofMode {
    const fn as_str(self) -> &'static str {
        match self {
            ProofMode::Core => "core",
            ProofMode::Compressed => "compressed",
            ProofMode::Plonk => "plonk",
        }
    }
}

impl From<ProofMode> for SP1ProofMode {
    fn from(value: ProofMode) -> Self {
        match value {
            ProofMode::Core => SP1ProofMode::Core,
            ProofMode::Compressed => SP1ProofMode::Compressed,
            ProofMode::Plonk => SP1ProofMode::Plonk,
        }
    }
}

impl ProofType {
    const fn as_raiko(self) -> RaikoProofType {
        match self {
            ProofType::Native => RaikoProofType::Native,
            ProofType::Risc0 => RaikoProofType::Risc0,
            ProofType::Sp1 => RaikoProofType::Sp1,
        }
    }
}

impl Stage {
    const fn as_str(self) -> &'static str {
        match self {
            Stage::Proposal => "proposal",
            Stage::OpcodeLab => "opcode-lab",
            Stage::RevmOpcodeLab => "revm-opcode-lab",
            Stage::PrecompileLab => "precompile-lab",
        }
    }
}

impl Args {
    fn effective_proof_mode(&self) -> ProofMode {
        self.proof_mode.unwrap_or({
            if self.aggregate.is_empty() {
                ProofMode::Compressed
            } else {
                ProofMode::Plonk
            }
        })
    }

    fn sp1_config(&self) -> Result<Sp1Config> {
        let prover = self.sp1_prover.map_or_else(
            || {
                if self.mode == Mode::Execute {
                    Sp1ProverMode::Local
                } else {
                    Sp1ProverMode::Network
                }
            },
            Into::into,
        );
        let proof_mode = self.effective_proof_mode();
        let config = Sp1Config {
            recursion: match proof_mode {
                ProofMode::Core => raiko2_prover::sp1::RecursionMode::Core,
                ProofMode::Compressed => raiko2_prover::sp1::RecursionMode::Compressed,
                ProofMode::Plonk => raiko2_prover::sp1::RecursionMode::Plonk,
            },
            prover,
            mode: match self.mode {
                Mode::Execute => raiko2_prover::sp1::ExecutionMode::Execute,
                Mode::Prove => raiko2_prover::sp1::ExecutionMode::Prove,
            },
            verify: true,
            network_mode: self.sp1_network_mode.into(),
            fulfillment_strategy: self.sp1_fulfillment_strategy.into(),
            skip_simulation: self.sp1_skip_simulation,
            cycle_limit: self.sp1_cycle_limit,
            proposal_cycle_limit: None,
            aggregation_cycle_limit: None,
            timeout_secs: self.sp1_timeout_secs,
            max_price_per_pgu: None,
            auction_timeout_secs: None,
            rpc_url: None,
            remote_verify: None,
        };
        config
            .validate()
            .map_err(anyhow::Error::msg)
            .map(|()| config)
    }
}

impl From<CliSp1ProverMode> for Sp1ProverMode {
    fn from(value: CliSp1ProverMode) -> Self {
        match value {
            CliSp1ProverMode::Mock => Self::Mock,
            CliSp1ProverMode::Local => Self::Local,
            CliSp1ProverMode::Network => Self::Network,
        }
    }
}

impl From<CliSp1NetworkMode> for Sp1NetworkMode {
    fn from(value: CliSp1NetworkMode) -> Self {
        match value {
            CliSp1NetworkMode::Reserved => Self::Reserved,
            CliSp1NetworkMode::Mainnet => Self::Mainnet,
        }
    }
}

impl From<CliSp1FulfillmentStrategy> for Sp1FulfillmentStrategy {
    fn from(value: CliSp1FulfillmentStrategy) -> Self {
        match value {
            CliSp1FulfillmentStrategy::Reserved => Self::Reserved,
            CliSp1FulfillmentStrategy::Hosted => Self::Hosted,
            CliSp1FulfillmentStrategy::Auction => Self::Auction,
        }
    }
}

fn count_entries(entries: impl Iterator<Item = (String, u64)>) -> Vec<BenchCountEntry> {
    let mut entries = entries
        .filter_map(|(label, count)| (count > 0).then_some(BenchCountEntry { label, count }))
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.label.cmp(&right.label));
    entries
}

fn current_memory_usage_kb() -> Option<(u64, u64)> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    let mut rss_kb = None;
    let mut hwm_kb = None;
    for line in status.lines() {
        if let Some(value) = line.strip_prefix("VmRSS:") {
            rss_kb = value.split_whitespace().next()?.parse::<u64>().ok();
        } else if let Some(value) = line.strip_prefix("VmHWM:") {
            hwm_kb = value.split_whitespace().next()?.parse::<u64>().ok();
        }
    }
    Some((rss_kb?, hwm_kb?))
}

fn record_memory_snapshot(report: &mut BenchReport, label: &'static str) {
    if let Some((rss_kb, hwm_kb)) = current_memory_usage_kb() {
        report.memory_snapshots.push(BenchMemoryEntry {
            label: label.to_string(),
            rss_kb,
            hwm_kb,
        });
    }
}

struct OpcodeLabMemoryLabels {
    start: &'static str,
    after_read_input: &'static str,
    after_stdin_write: &'static str,
    after_load_elf: &'static str,
    before_execute_run: &'static str,
    after_execute_run: &'static str,
    after_apply_execution_metadata: &'static str,
}

fn opcode_lab_memory_labels(stage: Stage) -> OpcodeLabMemoryLabels {
    match stage {
        Stage::OpcodeLab => OpcodeLabMemoryLabels {
            start: "opcode-lab:start",
            after_read_input: "opcode-lab:after_read_input",
            after_stdin_write: "opcode-lab:after_stdin_write",
            after_load_elf: "opcode-lab:after_load_elf",
            before_execute_run: "opcode-lab:before_execute_run",
            after_execute_run: "opcode-lab:after_execute_run",
            after_apply_execution_metadata: "opcode-lab:after_apply_execution_metadata",
        },
        Stage::RevmOpcodeLab => OpcodeLabMemoryLabels {
            start: "revm-opcode-lab:start",
            after_read_input: "revm-opcode-lab:after_read_input",
            after_stdin_write: "revm-opcode-lab:after_stdin_write",
            after_load_elf: "revm-opcode-lab:after_load_elf",
            before_execute_run: "revm-opcode-lab:before_execute_run",
            after_execute_run: "revm-opcode-lab:after_execute_run",
            after_apply_execution_metadata: "revm-opcode-lab:after_apply_execution_metadata",
        },
        Stage::Proposal | Stage::PrecompileLab => unreachable!("not an opcode lab stage"),
    }
}

fn apply_execution_metadata(report: &mut BenchReport, execution_report: &ExecutionReport) {
    report.exit_code = Some(execution_report.exit_code);
    let gas = execution_report.gas();
    let total_instruction_count = execution_report.total_instruction_count();
    let total_syscall_count = execution_report.total_syscall_count();
    let touched_memory_addresses = execution_report.touched_memory_addresses;
    report.gas = gas;
    report.total_instruction_count = Some(total_instruction_count);
    report.total_syscall_count = Some(total_syscall_count);
    report.touched_memory_addresses = Some(touched_memory_addresses);
    if let Some(gas) = gas {
        report.set_primary_workload_metric("prover_gas", gas);
    }
    report.push_workload_metric("sp1_total_instruction_count", total_instruction_count);
    report.push_workload_metric("sp1_total_syscall_count", total_syscall_count);
    report.push_workload_metric("sp1_touched_memory_addresses", touched_memory_addresses);
    report.invocation_tracker = count_entries(
        execution_report
            .invocation_tracker
            .iter()
            .map(|(label, count)| (label.clone(), *count)),
    );
    report.opcode_counts = count_entries(
        execution_report
            .opcode_counts
            .iter()
            .map(|(label, count)| (format!("{label:?}"), *count)),
    );
    report.syscall_counts = count_entries(
        execution_report
            .syscall_counts
            .iter()
            .map(|(label, count)| (format!("{label:?}"), *count)),
    );
}

fn apply_cycle_tracker(report: &mut BenchReport, execution_report: &ExecutionReport) {
    for (label, cycles) in &execution_report.cycle_tracker {
        report.cycle_tracker.push(BenchCycleEntry {
            label: label.clone(),
            cycles: *cycles,
        });
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Enable SP1 runtime logs (includes guest `println!` output).
    // Use `RUST_LOG=info` (or `debug`) when running this binary to see them.
    setup_logger();

    let args = Args::parse();

    if matches!(args.stage, Stage::OpcodeLab | Stage::RevmOpcodeLab) {
        return run_opcode_lab(args).await;
    }
    if args.stage == Stage::PrecompileLab {
        return run_precompile_lab(args).await;
    }
    if !args.aggregate.is_empty() {
        return run_aggregation(args).await;
    }
    run_proposal(args).await
}

fn read_input(path: &PathBuf, proof_type: ProofType) -> Result<GuestInput> {
    let contents = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let mut input: GuestInput = serde_json::from_str(&contents).context("parse input JSON")?;
    if !input.witnesses.is_empty() && input.proof_carry_data == ProofCarryData::default() {
        input.proof_carry_data = build_proof_carry_data(&input, proof_type.as_raiko())?;
    }
    Ok(input)
}

fn read_opcode_lab_input(path: &PathBuf) -> Result<OpcodeLabInput> {
    let contents = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_str(&contents).context("parse opcode-lab input JSON")
}

fn read_precompile_lab_input(path: &PathBuf) -> Result<PrecompileLabInput> {
    let contents = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_str(&contents).context("parse precompile-lab input JSON")
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum OpcodeLabInputList {
    Paths(Vec<PathBuf>),
    Object { inputs: Vec<PathBuf> },
}

fn read_opcode_lab_input_list(path: &PathBuf) -> Result<Vec<PathBuf>> {
    let contents = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let list: OpcodeLabInputList =
        serde_json::from_str(&contents).context("parse opcode-lab input list JSON")?;
    let paths = match list {
        OpcodeLabInputList::Paths(paths) => paths,
        OpcodeLabInputList::Object { inputs } => inputs,
    };
    if paths.is_empty() {
        anyhow::bail!("opcode-lab input list is empty");
    }
    Ok(paths)
}

async fn run_opcode_lab(args: Args) -> Result<()> {
    if args.proof_type != ProofType::Sp1 {
        anyhow::bail!(
            "{} is supported only for --proof-type sp1",
            args.stage.as_str()
        );
    }
    if args.mode != Mode::Execute {
        anyhow::bail!("{} supports only --mode execute", args.stage.as_str());
    }
    if !args.aggregate.is_empty() {
        anyhow::bail!(
            "{} does not support --aggregate proofs",
            args.stage.as_str()
        );
    }
    if args.input_list.is_some() {
        return run_opcode_lab_batch(args).await;
    }
    let input_path = args.input.clone().context("missing --input")?;
    let elf_path = args
        .elf
        .clone()
        .with_context(|| format!("missing --elf for {}", args.stage.as_str()))?;
    let proof_mode = args.effective_proof_mode();
    let mut report = BenchReport::new(
        args.stage.as_str(),
        args.mode.as_str(),
        proof_mode.as_str(),
        input_path.display().to_string(),
    );
    let labels = opcode_lab_memory_labels(args.stage);
    record_memory_snapshot(&mut report, labels.start);

    let input = read_opcode_lab_input(&input_path)?;
    record_memory_snapshot(&mut report, labels.after_read_input);
    let mut stdin = SP1Stdin::new();
    stdin.write(&input);
    record_memory_snapshot(&mut report, labels.after_stdin_write);
    let elf = fs::read(&elf_path).with_context(|| format!("read {}", elf_path.display()))?;
    record_memory_snapshot(&mut report, labels.after_load_elf);

    let sp1_config = args.sp1_config()?;
    let start = Instant::now();
    record_memory_snapshot(&mut report, labels.before_execute_run);
    let (public_values, execution_report) =
        execute_sp1_blocking(sp1_config.prover, elf, stdin).await?;
    record_memory_snapshot(&mut report, labels.after_execute_run);
    report.wall_time_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
    report.public_values = public_values.raw();
    apply_execution_metadata(&mut report, &execution_report);
    record_memory_snapshot(&mut report, labels.after_apply_execution_metadata);

    println!("public_values: {}", report.public_values);
    if !execution_report.cycle_tracker.is_empty() {
        println!("cycle_tracker:");
        for (label, cycles) in &execution_report.cycle_tracker {
            println!("  {label}: {cycles}");
        }
        apply_cycle_tracker(&mut report, &execution_report);
    }

    if let Some(path) = &args.json_out {
        let contents = serde_json::to_string_pretty(&report).context("serialize bench report")?;
        fs::write(path, contents).with_context(|| format!("write {}", path.display()))?;
    }

    Ok(())
}

async fn run_opcode_lab_batch(args: Args) -> Result<()> {
    let input_list_path = args.input_list.clone().context("missing --input-list")?;
    let jsonl_out_path = args
        .jsonl_out
        .clone()
        .with_context(|| format!("missing --jsonl-out for {} batch", args.stage.as_str()))?;
    let elf_path = args
        .elf
        .clone()
        .with_context(|| format!("missing --elf for {}", args.stage.as_str()))?;
    let proof_mode = args.effective_proof_mode();
    let input_paths = read_opcode_lab_input_list(&input_list_path)?;
    let mut inputs = Vec::with_capacity(input_paths.len());
    for input_path in input_paths {
        let input = read_opcode_lab_input(&input_path)?;
        inputs.push((input_path, input));
    }
    let elf = fs::read(&elf_path).with_context(|| format!("read {}", elf_path.display()))?;
    let sp1_config = args.sp1_config()?;
    let runs = execute_opcode_lab_batch_blocking(sp1_config.prover, elf, inputs).await?;

    let mut output = String::new();
    for run in runs {
        let mut report = BenchReport::new(
            args.stage.as_str(),
            args.mode.as_str(),
            proof_mode.as_str(),
            run.input_path.display().to_string(),
        );
        report.public_values = run.public_values;
        report.wall_time_ms = run.wall_time_ms;
        apply_execution_metadata(&mut report, &run.execution_report);
        apply_cycle_tracker(&mut report, &run.execution_report);
        println!(
            "input: {} public_values: {}",
            report.input, report.public_values
        );
        output.push_str(&serde_json::to_string(&report).context("serialize bench report")?);
        output.push('\n');
    }

    fs::write(&jsonl_out_path, output)
        .with_context(|| format!("write {}", jsonl_out_path.display()))?;
    Ok(())
}

async fn run_precompile_lab(args: Args) -> Result<()> {
    if args.proof_type != ProofType::Sp1 {
        anyhow::bail!("precompile-lab is supported only for --proof-type sp1");
    }
    if args.mode != Mode::Execute {
        anyhow::bail!("precompile-lab supports only --mode execute");
    }
    if !args.aggregate.is_empty() {
        anyhow::bail!("precompile-lab does not support --aggregate proofs");
    }
    if args.input_list.is_some() {
        return run_precompile_lab_batch(args).await;
    }
    let input_path = args.input.clone().context("missing --input")?;
    let elf_path = args
        .elf
        .clone()
        .context("missing --elf for precompile-lab")?;
    let proof_mode = args.effective_proof_mode();
    let mut report = BenchReport::new(
        args.stage.as_str(),
        args.mode.as_str(),
        proof_mode.as_str(),
        input_path.display().to_string(),
    );
    record_memory_snapshot(&mut report, "precompile-lab:start");

    let input = read_precompile_lab_input(&input_path)?;
    record_memory_snapshot(&mut report, "precompile-lab:after_read_input");
    let mut stdin = SP1Stdin::new();
    stdin.write(&input);
    record_memory_snapshot(&mut report, "precompile-lab:after_stdin_write");
    let elf = fs::read(&elf_path).with_context(|| format!("read {}", elf_path.display()))?;
    record_memory_snapshot(&mut report, "precompile-lab:after_load_elf");

    let sp1_config = args.sp1_config()?;
    let start = Instant::now();
    record_memory_snapshot(&mut report, "precompile-lab:before_execute_run");
    let (public_values, execution_report) =
        execute_sp1_blocking(sp1_config.prover, elf, stdin).await?;
    record_memory_snapshot(&mut report, "precompile-lab:after_execute_run");
    report.wall_time_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
    report.public_values = public_values.raw();
    apply_execution_metadata(&mut report, &execution_report);
    record_memory_snapshot(&mut report, "precompile-lab:after_apply_execution_metadata");

    println!("public_values: {}", report.public_values);
    if !execution_report.cycle_tracker.is_empty() {
        println!("cycle_tracker:");
        for (label, cycles) in &execution_report.cycle_tracker {
            println!("  {label}: {cycles}");
        }
        apply_cycle_tracker(&mut report, &execution_report);
    }

    if let Some(path) = &args.json_out {
        let contents = serde_json::to_string_pretty(&report).context("serialize bench report")?;
        fs::write(path, contents).with_context(|| format!("write {}", path.display()))?;
    }

    Ok(())
}

async fn run_precompile_lab_batch(args: Args) -> Result<()> {
    let input_list_path = args.input_list.clone().context("missing --input-list")?;
    let jsonl_out_path = args
        .jsonl_out
        .clone()
        .context("missing --jsonl-out for precompile-lab batch")?;
    let elf_path = args
        .elf
        .clone()
        .context("missing --elf for precompile-lab")?;
    let proof_mode = args.effective_proof_mode();
    let input_paths = read_opcode_lab_input_list(&input_list_path)?;
    let mut inputs = Vec::with_capacity(input_paths.len());
    for input_path in input_paths {
        let input = read_precompile_lab_input(&input_path)?;
        inputs.push((input_path, input));
    }
    let elf = fs::read(&elf_path).with_context(|| format!("read {}", elf_path.display()))?;
    let sp1_config = args.sp1_config()?;
    let runs = execute_precompile_lab_batch_blocking(sp1_config.prover, elf, inputs).await?;

    let mut output = String::new();
    for run in runs {
        let mut report = BenchReport::new(
            args.stage.as_str(),
            args.mode.as_str(),
            proof_mode.as_str(),
            run.input_path.display().to_string(),
        );
        report.public_values = run.public_values;
        report.wall_time_ms = run.wall_time_ms;
        apply_execution_metadata(&mut report, &run.execution_report);
        apply_cycle_tracker(&mut report, &run.execution_report);
        println!(
            "input: {} public_values: {}",
            report.input, report.public_values
        );
        output.push_str(&serde_json::to_string(&report).context("serialize bench report")?);
        output.push('\n');
    }

    fs::write(&jsonl_out_path, output)
        .with_context(|| format!("write {}", jsonl_out_path.display()))?;
    Ok(())
}

async fn run_proposal(args: Args) -> Result<()> {
    let input_path = args.input.clone().context("missing --input")?;
    let proof_mode = args.effective_proof_mode();
    let mut report = BenchReport::new(
        "proposal",
        args.mode.as_str(),
        proof_mode.as_str(),
        input_path.display().to_string(),
    );
    record_memory_snapshot(&mut report, "proposal:start");
    let input = read_input(&input_path, args.proof_type)?;
    record_memory_snapshot(&mut report, "proposal:after_read_input");

    match args.proof_type {
        ProofType::Sp1 => run_sp1_proposal(args, input_path, input, report).await,
        ProofType::Native => run_native_proposal(args, input_path, input, report).await,
        ProofType::Risc0 => run_risc0_proposal(args, input_path, input, report).await,
    }
}

async fn run_aggregation(args: Args) -> Result<()> {
    if args.proof_type != ProofType::Sp1 {
        anyhow::bail!("aggregation is supported only for --proof-type sp1");
    }
    if args.aggregate.is_empty() {
        anyhow::bail!("missing --aggregate proofs");
    }
    let sp1_config = args.sp1_config()?;
    let output_path = args
        .output
        .as_ref()
        .context("missing --output for aggregation")?;
    if args.mode == Mode::Execute {
        anyhow::bail!("aggregation requires --mode prove");
    }
    let proof_mode = args.effective_proof_mode();
    if proof_mode != ProofMode::Plonk {
        anyhow::bail!("aggregation proof output requires --proof-mode plonk");
    }

    let proofs = read_proofs(&args.aggregate)?;
    let (aggregation_input, sp1_proofs, image_id) = build_aggregation_inputs(&proofs)?;

    let mut stdin = SP1Stdin::new();
    stdin.write(&aggregation_input);

    let backend = load_sp1_shasta_backend()
        .map_err(anyhow::Error::msg)
        .context("load SP1 Shasta guest ELFs")?;
    let proposal_elf = backend
        .elf(ProofStage::Proposal)
        .context("load SP1 proposal ELF")?;
    let elf = backend
        .elf(ProofStage::Aggregation)
        .context("load SP1 aggregation ELF")?;
    let start = Instant::now();
    let (proof, proposal_vk) = match sp1_config.prover {
        Sp1ProverMode::Mock => {
            let prover = BlockingProverClient::builder().mock().build();
            let proposal_pk = setup_sp1_pk(&prover, proposal_elf, "proposal")?;
            let proposal_vk = proposal_pk.verifying_key().clone();
            for proof in sp1_proofs {
                match proof {
                    SP1Proof::Compressed(reduce_proof) => {
                        stdin.write_proof(*reduce_proof, proposal_vk.vk.clone());
                    }
                    _ => anyhow::bail!("aggregation requires compressed proofs"),
                }
            }
            (
                prove_sp1_local(&prover, elf, stdin, proof_mode.into(), None, "aggregation")?,
                proposal_vk,
            )
        }
        Sp1ProverMode::Local => {
            let prover = BlockingProverClient::builder().cpu().build();
            let proposal_pk = setup_sp1_pk(&prover, proposal_elf, "proposal")?;
            let proposal_vk = proposal_pk.verifying_key().clone();
            for proof in sp1_proofs {
                match proof {
                    SP1Proof::Compressed(reduce_proof) => {
                        stdin.write_proof(*reduce_proof, proposal_vk.vk.clone());
                    }
                    _ => anyhow::bail!("aggregation requires compressed proofs"),
                }
            }
            (
                prove_sp1_local(&prover, elf, stdin, proof_mode.into(), None, "aggregation")?,
                proposal_vk,
            )
        }
        Sp1ProverMode::Network => {
            let prover = build_sp1_network_prover(&sp1_config).await?;
            let proposal_pk = prover
                .setup(proposal_elf.into())
                .await
                .context("setup SP1 proposal ELF")?;
            let proposal_vk = proposal_pk.verifying_key().clone();
            for proof in sp1_proofs {
                match proof {
                    SP1Proof::Compressed(reduce_proof) => {
                        stdin.write_proof(*reduce_proof, proposal_vk.vk.clone());
                    }
                    _ => anyhow::bail!("aggregation requires compressed proofs"),
                }
            }
            let pk = prover
                .setup(elf.into())
                .await
                .context("setup SP1 aggregation ELF")?;
            (
                request_network_proof(&prover, &pk, stdin, proof_mode.into(), &sp1_config)
                    .await
                    .context("prove failed")?,
                proposal_vk,
            )
        }
    };
    let mut report = BenchReport::new(
        "aggregation",
        args.mode.as_str(),
        proof_mode.as_str(),
        output_path.display().to_string(),
    );
    report.public_values = proof.proof.public_values.raw();
    report.wall_time_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);

    let output = build_sp1_aggregation_output(&proof.proof, proof.vkey(), &proposal_vk)?;
    write_proof_json(output_path, &output)?;

    println!("public_values: {}", report.public_values);

    if let Some(path) = &args.json_out {
        let contents = serde_json::to_string_pretty(&report).context("serialize bench report")?;
        fs::write(path, contents).with_context(|| format!("write {}", path.display()))?;
    }

    println!("image_id: 0x{}", hex::encode(words_to_bytes_be(&image_id)));

    Ok(())
}

fn build_sp1_output(
    proof: &SP1ProofWithPublicValues,
    vk: &SP1VerifyingKey,
    proof_payload: Option<String>,
    carry: Option<&ProofCarryData>,
) -> Result<Proof> {
    let public_values = proof.public_values.as_slice();
    if public_values.len() < 32 {
        bail!(
            "SP1 public values must contain at least 32 bytes, got {}",
            public_values.len()
        );
    }
    let input_hash = B256::from_slice(&public_values[..32]);
    let extra_data = match carry {
        Some(data) => Some(encode_proof_carry_data(data)?),
        None => None,
    };
    let quote = serde_json::to_string(&proof.proof).context("serialize SP1 quote")?;

    Ok(Proof {
        proof: proof_payload,
        input: Some(input_hash),
        quote: Some(quote),
        uuid: Some(sp1_vk_uuid(vk)),
        extra_data,
        ..Default::default()
    })
}

fn build_sp1_proposal_output(
    proof: &SP1ProofWithPublicValues,
    vk: &SP1VerifyingKey,
    carry: Option<&ProofCarryData>,
) -> Result<Proof> {
    build_sp1_output(
        proof,
        vk,
        encode_sp1_proposal_proof_payload(proof, vk),
        carry,
    )
}

fn build_sp1_aggregation_output(
    proof: &SP1ProofWithPublicValues,
    aggregation_vk: &SP1VerifyingKey,
    block_vk: &SP1VerifyingKey,
) -> Result<Proof> {
    build_sp1_output(
        proof,
        aggregation_vk,
        encode_sp1_aggregation_proof_payload(proof, aggregation_vk, block_vk),
        None,
    )
}

#[derive(Clone)]
struct Sp1ProofOutput {
    proof: SP1ProofWithPublicValues,
    vkey: SP1VerifyingKey,
}

impl Sp1ProofOutput {
    fn vkey(&self) -> &SP1VerifyingKey {
        &self.vkey
    }
}

fn setup_sp1_pk<P>(prover: &P, elf: &[u8], label: &str) -> Result<SP1ProvingKey>
where
    P: BlockingProver<ProvingKey = SP1ProvingKey>,
{
    prover
        .setup(elf.into())
        .map_err(|err| anyhow::anyhow!("setup SP1 {label} ELF: {err:?}"))
}

fn prove_sp1_local<P>(
    prover: &P,
    elf: &[u8],
    stdin: SP1Stdin,
    proof_mode: SP1ProofMode,
    cycle_limit: Option<u64>,
    label: &str,
) -> Result<Sp1ProofOutput>
where
    P: BlockingProver<ProvingKey = SP1ProvingKey>,
{
    let pk = setup_sp1_pk(prover, elf, label)?;
    let vkey = pk.verifying_key().clone();
    prove_sp1_with_pk(prover, &pk, vkey, stdin, proof_mode, cycle_limit)
}

fn prove_sp1_with_pk<P>(
    prover: &P,
    pk: &SP1ProvingKey,
    vkey: SP1VerifyingKey,
    stdin: SP1Stdin,
    proof_mode: SP1ProofMode,
    cycle_limit: Option<u64>,
) -> Result<Sp1ProofOutput>
where
    P: BlockingProver<ProvingKey = SP1ProvingKey>,
{
    let mut request = prover.prove(pk, stdin).mode(proof_mode);
    if let Some(cycle_limit) = cycle_limit {
        request = request.cycle_limit(cycle_limit);
    }
    let proof = request
        .run()
        .map_err(|err| anyhow::anyhow!("prove failed: {err:?}"))?;
    Ok(Sp1ProofOutput { proof, vkey })
}

fn execute_sp1_local<P>(
    prover: &P,
    elf: &[u8],
    stdin: SP1Stdin,
) -> Result<(sp1_sdk::SP1PublicValues, ExecutionReport)>
where
    P: BlockingProver<ProvingKey = SP1ProvingKey>,
{
    prover
        .execute(elf.into(), stdin)
        .run()
        .map_err(|err| anyhow::anyhow!("execute failed: {err:?}"))
}

async fn execute_sp1_blocking(
    prover_mode: Sp1ProverMode,
    elf: Vec<u8>,
    stdin: SP1Stdin,
) -> Result<(sp1_sdk::SP1PublicValues, ExecutionReport)> {
    tokio::task::spawn_blocking(move || match prover_mode {
        Sp1ProverMode::Mock => {
            let prover = BlockingProverClient::builder().mock().build();
            execute_sp1_local(&prover, &elf, stdin)
        }
        Sp1ProverMode::Local => {
            let prover = BlockingProverClient::builder().cpu().build();
            execute_sp1_local(&prover, &elf, stdin)
        }
        Sp1ProverMode::Network => {
            anyhow::bail!("sp1.mode=execute does not support sp1.prover=network")
        }
    })
    .await
    .context("join SP1 blocking execute task")?
}

struct OpcodeLabExecution {
    input_path: PathBuf,
    public_values: String,
    wall_time_ms: u64,
    execution_report: ExecutionReport,
}

struct Risc0ProposalExecution {
    public_values: String,
    wall_time_ms: u64,
    user_cycles: u64,
    padded_cycles: u64,
    segment_count: u64,
    po2_counts: Vec<BenchCountEntry>,
}

fn risc0_padded_cycles(po2_values: impl IntoIterator<Item = u32>) -> u64 {
    po2_values
        .into_iter()
        .map(|po2| 1u64.checked_shl(po2).unwrap_or(u64::MAX))
        .sum()
}

fn apply_risc0_execution_metadata(report: &mut BenchReport, execution: &Risc0ProposalExecution) {
    report.public_values = execution.public_values.clone();
    report.wall_time_ms = execution.wall_time_ms;
    report.risc0_user_cycles = Some(execution.user_cycles);
    report.risc0_padded_cycles = Some(execution.padded_cycles);
    report.risc0_segment_count = Some(execution.segment_count);
    report.risc0_po2_counts = execution.po2_counts.clone();
    report.set_primary_workload_metric("risc0_padded_cycles", execution.padded_cycles);
    report.push_workload_metric("risc0_user_cycles", execution.user_cycles);
    report.push_workload_metric("risc0_segment_count", execution.segment_count);
}

async fn execute_risc0_proposal_blocking(
    input: GuestInput,
    elf: Vec<u8>,
    execution_po2: u32,
) -> Result<Risc0ProposalExecution> {
    tokio::task::spawn_blocking(move || {
        let encoded = bincode::serialize(&input).context("serialize RISC0 guest input")?;
        let mut env_builder = risc0_zkvm::ExecutorEnv::builder();
        env_builder
            .write_frame(encoded.as_slice())
            .segment_limit_po2(execution_po2);
        let env = env_builder
            .build()
            .map_err(|err| anyhow::anyhow!("build RISC0 executor env: {err}"))?;
        let start = Instant::now();
        let session = risc0_zkvm::local_executor()
            .execute(env, &elf)
            .map_err(|err| anyhow::anyhow!("execute RISC0 proposal dry-run: {err}"))?;
        let wall_time_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
        let user_cycles = session.cycles();
        let po2_values = session
            .segments
            .iter()
            .map(|segment| segment.po2)
            .collect::<Vec<_>>();
        let padded_cycles = risc0_padded_cycles(po2_values.iter().copied());
        let mut po2_histogram = BTreeMap::<u32, u64>::new();
        for po2 in po2_values {
            *po2_histogram.entry(po2).or_default() += 1;
        }
        let po2_counts = po2_histogram
            .into_iter()
            .map(|(po2, count)| BenchCountEntry {
                label: po2.to_string(),
                count,
            })
            .collect::<Vec<_>>();
        let public_values = hex::encode_prefixed(&session.journal.bytes);
        Ok(Risc0ProposalExecution {
            public_values,
            wall_time_ms,
            user_cycles,
            padded_cycles,
            segment_count: u64::try_from(session.segments.len()).unwrap_or(u64::MAX),
            po2_counts,
        })
    })
    .await
    .context("join RISC0 blocking execute task")?
}

async fn execute_opcode_lab_batch_blocking(
    prover_mode: Sp1ProverMode,
    elf: Vec<u8>,
    inputs: Vec<(PathBuf, OpcodeLabInput)>,
) -> Result<Vec<OpcodeLabExecution>> {
    tokio::task::spawn_blocking(move || match prover_mode {
        Sp1ProverMode::Mock => {
            let prover = BlockingProverClient::builder().mock().build();
            execute_opcode_lab_batch_local(&prover, &elf, inputs)
        }
        Sp1ProverMode::Local => {
            let prover = BlockingProverClient::builder().cpu().build();
            execute_opcode_lab_batch_local(&prover, &elf, inputs)
        }
        Sp1ProverMode::Network => {
            anyhow::bail!("sp1.mode=execute does not support sp1.prover=network")
        }
    })
    .await
    .context("join SP1 blocking opcode-lab batch task")?
}

fn execute_opcode_lab_batch_local<P>(
    prover: &P,
    elf: &[u8],
    inputs: Vec<(PathBuf, OpcodeLabInput)>,
) -> Result<Vec<OpcodeLabExecution>>
where
    P: BlockingProver<ProvingKey = SP1ProvingKey>,
{
    let mut outputs = Vec::with_capacity(inputs.len());
    for (input_path, input) in inputs {
        let mut stdin = SP1Stdin::new();
        stdin.write(&input);
        let start = Instant::now();
        let (public_values, execution_report) = execute_sp1_local(prover, elf, stdin)?;
        outputs.push(OpcodeLabExecution {
            input_path,
            public_values: public_values.raw(),
            wall_time_ms: u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX),
            execution_report,
        });
    }
    Ok(outputs)
}

async fn execute_precompile_lab_batch_blocking(
    prover_mode: Sp1ProverMode,
    elf: Vec<u8>,
    inputs: Vec<(PathBuf, PrecompileLabInput)>,
) -> Result<Vec<OpcodeLabExecution>> {
    tokio::task::spawn_blocking(move || match prover_mode {
        Sp1ProverMode::Mock => {
            let prover = BlockingProverClient::builder().mock().build();
            execute_precompile_lab_batch_local(&prover, &elf, inputs)
        }
        Sp1ProverMode::Local => {
            let prover = BlockingProverClient::builder().cpu().build();
            execute_precompile_lab_batch_local(&prover, &elf, inputs)
        }
        Sp1ProverMode::Network => {
            anyhow::bail!("sp1.mode=execute does not support sp1.prover=network")
        }
    })
    .await
    .context("join SP1 blocking precompile-lab batch task")?
}

fn execute_precompile_lab_batch_local<P>(
    prover: &P,
    elf: &[u8],
    inputs: Vec<(PathBuf, PrecompileLabInput)>,
) -> Result<Vec<OpcodeLabExecution>>
where
    P: BlockingProver<ProvingKey = SP1ProvingKey>,
{
    let mut outputs = Vec::with_capacity(inputs.len());
    for (input_path, input) in inputs {
        let mut stdin = SP1Stdin::new();
        stdin.write(&input);
        let start = Instant::now();
        let (public_values, execution_report) = execute_sp1_local(prover, elf, stdin)?;
        outputs.push(OpcodeLabExecution {
            input_path,
            public_values: public_values.raw(),
            wall_time_ms: u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX),
            execution_report,
        });
    }
    Ok(outputs)
}

async fn build_sp1_network_prover(config: &Sp1Config) -> Result<NetworkProver> {
    let private_key = std::env::var("NETWORK_PRIVATE_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .context("NETWORK_PRIVATE_KEY must be set for sp1 network proving")?;
    let signer = NetworkSigner::local(&private_key)
        .context("NETWORK_PRIVATE_KEY is not a valid SP1 network signer")?;
    let network_mode = sp1_sdk_network_mode(config.network_mode);
    let rpc_url = config
        .rpc_url
        .clone()
        .unwrap_or_else(|| get_default_rpc_url_for_mode(network_mode).to_string());
    Ok(NetworkProver::new(signer, &rpc_url, network_mode).await)
}

const fn sp1_sdk_network_mode(mode: Sp1NetworkMode) -> Sp1SdkNetworkMode {
    match mode {
        Sp1NetworkMode::Mainnet => Sp1SdkNetworkMode::Mainnet,
        Sp1NetworkMode::Reserved => Sp1SdkNetworkMode::Reserved,
    }
}

async fn request_network_proof(
    prover: &NetworkProver,
    pk: &SP1ProvingKey,
    stdin: SP1Stdin,
    proof_mode: SP1ProofMode,
    config: &Sp1Config,
) -> Result<Sp1ProofOutput> {
    let timeout = Duration::from_secs(config.timeout_secs);
    let mut request = prover
        .prove(pk, stdin)
        .mode(proof_mode)
        .strategy(config.fulfillment_strategy.into())
        .skip_simulation(config.skip_simulation)
        .cycle_limit(config.cycle_limit)
        .timeout(timeout);
    if let Some(max_price_per_pgu) = config.max_price_per_pgu {
        request = request.max_price_per_pgu(max_price_per_pgu);
    }
    let request_id = request
        .request()
        .await
        .context("request SP1 network proof")?;
    eprintln!("sp1 request_id: {request_id}");
    let proof = prover
        .wait_proof(request_id, Some(timeout), None)
        .await
        .context("wait for SP1 network proof")?;
    Ok(Sp1ProofOutput {
        proof,
        vkey: pk.verifying_key().clone(),
    })
}

async fn run_sp1_proposal(
    args: Args,
    input_path: PathBuf,
    input: GuestInput,
    mut report: BenchReport,
) -> Result<()> {
    let sp1_config = args.sp1_config()?;
    let proof_mode = args.effective_proof_mode();
    record_memory_snapshot(&mut report, "proposal:before_stdin_write");
    let mut stdin = SP1Stdin::new();
    stdin.write(&input);
    record_memory_snapshot(&mut report, "proposal:after_stdin_write");

    let elf = load_sp1_proposal_elf(&args)?;
    report.input = input_path.display().to_string();
    record_memory_snapshot(&mut report, "proposal:after_load_elf");

    match args.mode {
        Mode::Execute => {
            let start = Instant::now();
            record_memory_snapshot(&mut report, "proposal:before_execute_run");
            let (public_values, execution_report) =
                execute_sp1_blocking(sp1_config.prover, elf.clone(), stdin).await?;
            record_memory_snapshot(&mut report, "proposal:after_execute_run");
            report.wall_time_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
            report.public_values = public_values.raw();
            apply_execution_metadata(&mut report, &execution_report);
            record_memory_snapshot(&mut report, "proposal:after_apply_execution_metadata");

            println!("public_values: {}", report.public_values);
            if !execution_report.cycle_tracker.is_empty() {
                println!("cycle_tracker:");
                for (label, cycles) in &execution_report.cycle_tracker {
                    println!("  {label}: {cycles}");
                }
                apply_cycle_tracker(&mut report, &execution_report);
            }
        }
        Mode::Prove => {
            let start = Instant::now();
            match sp1_config.prover {
                Sp1ProverMode::Mock => {
                    let prover = BlockingProverClient::builder().mock().build();
                    record_memory_snapshot(&mut report, "proposal:before_setup");
                    let pk = setup_sp1_pk(&prover, &elf, "proposal")?;
                    let vkey = pk.verifying_key().clone();
                    record_memory_snapshot(&mut report, "proposal:after_setup");
                    let output = prove_sp1_with_pk(
                        &prover,
                        &pk,
                        vkey,
                        stdin,
                        proof_mode.into(),
                        Some(sp1_config.cycle_limit),
                    )?;
                    record_memory_snapshot(&mut report, "proposal:after_prove_run");
                    report.wall_time_ms =
                        u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
                    report.public_values = output.proof.public_values.raw();
                    println!("public_values: {}", report.public_values);

                    if let Some(path) = &args.output {
                        let proof = build_sp1_proposal_output(
                            &output.proof,
                            output.vkey(),
                            Some(&input.proof_carry_data),
                        )?;
                        write_proof_json(path, &proof)?;
                    }
                }
                Sp1ProverMode::Local => {
                    let prover = BlockingProverClient::builder().cpu().build();
                    record_memory_snapshot(&mut report, "proposal:before_setup");
                    let pk = setup_sp1_pk(&prover, &elf, "proposal")?;
                    let vkey = pk.verifying_key().clone();
                    record_memory_snapshot(&mut report, "proposal:after_setup");
                    let output = prove_sp1_with_pk(
                        &prover,
                        &pk,
                        vkey,
                        stdin,
                        proof_mode.into(),
                        Some(sp1_config.cycle_limit),
                    )?;
                    record_memory_snapshot(&mut report, "proposal:after_prove_run");
                    report.wall_time_ms =
                        u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
                    report.public_values = output.proof.public_values.raw();
                    println!("public_values: {}", report.public_values);

                    if let Some(path) = &args.output {
                        let proof = build_sp1_proposal_output(
                            &output.proof,
                            output.vkey(),
                            Some(&input.proof_carry_data),
                        )?;
                        write_proof_json(path, &proof)?;
                    }
                }
                Sp1ProverMode::Network => {
                    let prover = build_sp1_network_prover(&sp1_config).await?;
                    record_memory_snapshot(&mut report, "proposal:before_setup");
                    let pk = prover
                        .setup(elf.into())
                        .await
                        .context("setup SP1 proposal ELF")?;
                    record_memory_snapshot(&mut report, "proposal:after_setup");
                    let output =
                        request_network_proof(&prover, &pk, stdin, proof_mode.into(), &sp1_config)
                            .await
                            .context("prove failed")?;
                    record_memory_snapshot(&mut report, "proposal:after_network_proof");
                    report.wall_time_ms =
                        u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
                    report.public_values = output.proof.public_values.raw();
                    println!("public_values: {}", report.public_values);

                    if let Some(path) = &args.output {
                        let output = build_sp1_proposal_output(
                            &output.proof,
                            output.vkey(),
                            Some(&input.proof_carry_data),
                        )?;
                        write_proof_json(path, &output)?;
                    }
                }
            }
        }
    }

    if let Some(path) = &args.json_out {
        let contents = serde_json::to_string_pretty(&report).context("serialize bench report")?;
        fs::write(path, contents).with_context(|| format!("write {}", path.display()))?;
    }

    Ok(())
}

fn load_sp1_proposal_elf(args: &Args) -> Result<Vec<u8>> {
    if let Some(path) = &args.elf {
        return fs::read(path).with_context(|| format!("read {}", path.display()));
    }

    let backend = load_sp1_shasta_backend()
        .map_err(anyhow::Error::msg)
        .context("load SP1 Shasta guest ELFs")?;
    backend
        .elf(ProofStage::Proposal)
        .context("load SP1 proposal ELF")
        .map(ToOwned::to_owned)
}

async fn run_risc0_proposal(
    args: Args,
    input_path: PathBuf,
    input: GuestInput,
    mut report: BenchReport,
) -> Result<()> {
    if args.mode != Mode::Execute {
        anyhow::bail!("guest-launcher RISC0 proposal currently supports only --mode execute");
    }
    if !args.aggregate.is_empty() {
        anyhow::bail!("RISC0 proposal execute does not support --aggregate proofs");
    }

    let elf = if let Some(path) = &args.elf {
        fs::read(path).with_context(|| format!("read {}", path.display()))?
    } else {
        let backend = load_risc0_shasta_backend()
            .map_err(anyhow::Error::msg)
            .context("load RISC0 Shasta guest ELFs")?;
        backend
            .elf(ProofStage::Proposal)
            .context("load RISC0 proposal ELF")?
            .to_vec()
    };
    report.input = input_path.display().to_string();
    record_memory_snapshot(&mut report, "proposal:risc0_after_load_elf");

    let execution = execute_risc0_proposal_blocking(input, elf, args.risc0_execution_po2).await?;
    apply_risc0_execution_metadata(&mut report, &execution);
    record_memory_snapshot(&mut report, "proposal:risc0_after_execute_run");

    println!("public_values: {}", report.public_values);
    println!("risc0_user_cycles: {}", execution.user_cycles);
    println!("risc0_padded_cycles: {}", execution.padded_cycles);
    println!("risc0_segment_count: {}", execution.segment_count);

    if let Some(path) = &args.json_out {
        let contents = serde_json::to_string_pretty(&report).context("serialize bench report")?;
        fs::write(path, contents).with_context(|| format!("write {}", path.display()))?;
    }

    Ok(())
}

async fn run_native_proposal(
    args: Args,
    input_path: PathBuf,
    input: GuestInput,
    mut report: BenchReport,
) -> Result<()> {
    if args.mode == Mode::Execute {
        anyhow::bail!("native backend does not support --mode execute");
    }
    let output_path = args
        .output
        .as_ref()
        .context("missing --output for native prove")?;

    let backend = NativeBackend;
    let prover = NativeProver;

    let start = Instant::now();
    let proof = prover
        .prove(input, &serde_json::Value::Null, &backend)
        .await
        .context("native prove failed")?;
    record_memory_snapshot(&mut report, "proposal:after_native_prove");
    let wall_time_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);

    write_proof_json(output_path, &proof)?;

    if let Some(input_hash) = proof.input {
        println!("public_values: {input_hash:#x}");
    }

    if let Some(path) = &args.json_out {
        report.input = input_path.display().to_string();
        report.public_values = proof
            .input
            .map(|h| format!("{h:#x}"))
            .unwrap_or_else(String::new);
        report.wall_time_ms = wall_time_ms;
        let contents = serde_json::to_string_pretty(&report).context("serialize bench report")?;
        fs::write(path, contents).with_context(|| format!("write {}", path.display()))?;
    }

    Ok(())
}

fn read_proofs(paths: &[PathBuf]) -> Result<Vec<Proof>> {
    let mut proofs = Vec::with_capacity(paths.len());
    for path in paths {
        let contents =
            fs::read_to_string(path).with_context(|| format!("read proof {}", path.display()))?;
        let proof: Proof =
            serde_json::from_str(&contents).with_context(|| format!("parse {}", path.display()))?;
        proofs.push(proof);
    }
    Ok(proofs)
}

fn build_aggregation_inputs(
    proofs: &[Proof],
) -> Result<(ShastaZkAggregationGuestInput, Vec<SP1Proof>, [u32; 8])> {
    let mut proof_carry_data_vec = Vec::with_capacity(proofs.len());
    let mut block_inputs = Vec::with_capacity(proofs.len());
    let mut sp1_proofs = Vec::with_capacity(proofs.len());
    let mut image_id: Option<[u32; 8]> = None;

    for proof in proofs {
        let carry_value = proof
            .extra_data
            .as_ref()
            .context("missing proof extra_data")?;
        let carry = decode_proof_carry_data(carry_value)?;
        let expected_input = hash_shasta_subproof_input(&carry);
        if let Some(input_hash) = proof.input
            && input_hash != expected_input
        {
            anyhow::bail!("proof input hash does not match shasta carry data");
        }
        block_inputs.push(expected_input);
        proof_carry_data_vec.push(carry);

        let uuid = proof.uuid.as_deref().context("missing proof uuid")?;
        let candidate_id = parse_image_id_from_uuid(uuid)?;
        if let Some(existing) = image_id {
            if existing != candidate_id {
                anyhow::bail!("mismatched proof image ids");
            }
        } else {
            image_id = Some(candidate_id);
        }

        let sp1_proof = load_sp1_subproof_for_aggregation(proof)
            .map_err(|err| anyhow::anyhow!("load SP1 aggregation subproof: {err}"))?;
        sp1_proofs.push(sp1_proof);
    }

    let image_id = image_id.context("missing proof image id")?;
    let aggregation_input = ShastaZkAggregationGuestInput {
        image_id,
        block_inputs,
        proof_carry_data_vec,
        prover_address: Address::default(),
    };

    Ok((aggregation_input, sp1_proofs, image_id))
}

fn parse_image_id_from_uuid(uuid: &str) -> Result<[u32; 8]> {
    sp1_image_id_words_from_uuid(uuid).map_err(anyhow::Error::msg)
}

fn write_proof_json(path: &PathBuf, proof: &Proof) -> Result<()> {
    let contents = serde_json::to_string_pretty(proof).context("serialize proof json")?;
    fs::write(path, contents).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        Args, ProofType, Stage, load_sp1_proposal_elf, parse_image_id_from_uuid, read_input,
        read_opcode_lab_input_list, risc0_padded_cycles,
    };
    use alloy_primitives::{Address, B256};
    use clap::Parser as _;
    use raiko2_primitives::{ProofType as RaikoProofType, SupportedChainSpecs};
    use raiko2_primitives_shasta::{GuestInput, build_proof_carry_data};
    use std::fs;

    #[test]
    fn parses_image_id_from_uuid_hex() {
        let uuid = "0x000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
        let image_id = parse_image_id_from_uuid(uuid).expect("parse image id");
        assert_eq!(
            image_id,
            [
                0x03020100, 0x07060504, 0x0b0a0908, 0x0f0e0d0c, 0x13121110, 0x17161514, 0x1b1a1918,
                0x1f1e1d1c
            ]
        );
    }

    #[test]
    fn parses_opcode_lab_stage_with_explicit_elf() {
        let args = Args::try_parse_from([
            "guest-launcher",
            "--stage",
            "opcode-lab",
            "--proof-type",
            "sp1",
            "--mode",
            "execute",
            "--sp1-prover",
            "local",
            "--elf",
            "crates/guests/elf/sp1_opcode_lab.elf",
            "--input",
            "/tmp/opcode-lab.json",
        ])
        .expect("parse args");

        assert_eq!(args.stage, Stage::OpcodeLab);
        assert_eq!(
            args.elf.expect("elf path").display().to_string(),
            "crates/guests/elf/sp1_opcode_lab.elf"
        );
    }

    #[test]
    fn parses_opcode_lab_batch_input_list() {
        let args = Args::try_parse_from([
            "guest-launcher",
            "--stage",
            "opcode-lab",
            "--proof-type",
            "sp1",
            "--mode",
            "execute",
            "--sp1-prover",
            "local",
            "--elf",
            "crates/guests/elf/sp1_opcode_lab.elf",
            "--input-list",
            "/tmp/opcode-lab-inputs.json",
            "--jsonl-out",
            "/tmp/opcode-lab-reports.jsonl",
        ])
        .expect("parse args");

        assert_eq!(args.stage, Stage::OpcodeLab);
        assert_eq!(
            args.input_list.expect("input list").display().to_string(),
            "/tmp/opcode-lab-inputs.json"
        );
        assert_eq!(
            args.jsonl_out.expect("jsonl out").display().to_string(),
            "/tmp/opcode-lab-reports.jsonl"
        );
    }

    #[test]
    fn parses_revm_opcode_lab_stage_with_explicit_elf() {
        let args = Args::try_parse_from([
            "guest-launcher",
            "--stage",
            "revm-opcode-lab",
            "--proof-type",
            "sp1",
            "--mode",
            "execute",
            "--sp1-prover",
            "local",
            "--elf",
            "crates/guests/elf/sp1_revm_opcode_lab.elf",
            "--input",
            "/tmp/revm-opcode-lab.json",
        ])
        .expect("parse args");

        assert_eq!(args.stage, Stage::RevmOpcodeLab);
        assert_eq!(
            args.elf.expect("elf path").display().to_string(),
            "crates/guests/elf/sp1_revm_opcode_lab.elf"
        );
    }

    #[test]
    fn parses_precompile_lab_batch_input_list() {
        let args = Args::try_parse_from([
            "guest-launcher",
            "--stage",
            "precompile-lab",
            "--proof-type",
            "sp1",
            "--mode",
            "execute",
            "--sp1-prover",
            "local",
            "--elf",
            "crates/guests/elf/sp1_precompile_lab.elf",
            "--input-list",
            "/tmp/precompile-lab-inputs.json",
            "--jsonl-out",
            "/tmp/precompile-lab-reports.jsonl",
        ])
        .expect("parse args");

        assert_eq!(args.stage, Stage::PrecompileLab);
        assert_eq!(
            args.elf.expect("elf path").display().to_string(),
            "crates/guests/elf/sp1_precompile_lab.elf"
        );
    }

    #[test]
    fn parses_risc0_execute_proposal() {
        let args = Args::try_parse_from([
            "guest-launcher",
            "--stage",
            "proposal",
            "--proof-type",
            "risc0",
            "--mode",
            "execute",
            "--input",
            "/tmp/guest-input.json",
            "--json-out",
            "/tmp/risc0-report.json",
        ])
        .expect("parse args");

        assert_eq!(args.proof_type, ProofType::Risc0);
        assert_eq!(args.stage, Stage::Proposal);
    }

    #[test]
    fn sp1_proposal_elf_uses_explicit_elf_path() {
        let elf_path = temp_input_path("sp1-proposal-elf");
        let expected = vec![0xde, 0xad, 0xbe, 0xef];
        fs::write(&elf_path, &expected).expect("write elf");
        let args = Args::try_parse_from([
            "guest-launcher",
            "--stage",
            "proposal",
            "--proof-type",
            "sp1",
            "--mode",
            "execute",
            "--sp1-prover",
            "local",
            "--elf",
            elf_path.to_str().expect("utf8 path"),
            "--input",
            "/tmp/guest-input.json",
        ])
        .expect("parse args");

        let elf = load_sp1_proposal_elf(&args).expect("load explicit elf");

        fs::remove_file(elf_path).expect("cleanup temp elf");
        assert_eq!(elf, expected);
    }

    #[test]
    fn risc0_padded_cycles_sum_segment_po2s() {
        assert_eq!(risc0_padded_cycles([10, 11, 10]), 4096);
    }

    #[test]
    fn read_opcode_lab_input_list_accepts_json_path_array() {
        let path = temp_input_path("opcode-lab-input-list");
        fs::write(
            &path,
            r#"[
              "/tmp/add-count-0.json",
              "/tmp/add-count-4.json"
            ]"#,
        )
        .expect("write input list");

        let inputs = read_opcode_lab_input_list(&path).expect("read input list");

        assert_eq!(
            inputs,
            vec![
                std::path::PathBuf::from("/tmp/add-count-0.json"),
                std::path::PathBuf::from("/tmp/add-count-4.json"),
            ]
        );

        let _ = fs::remove_file(path);
    }

    fn sample_guest_input() -> GuestInput {
        let mut input = GuestInput::default();
        input.taiko.proposal_id = 7;
        input.taiko.chain_spec.name = "taiko_mainnet".to_string();
        input.taiko.chain_spec.chain_id = 167_000;
        input.taiko.chain_spec.is_taiko = true;
        input.taiko.prover_data.actual_prover = Address::from([0x11; 20]);
        input.taiko.proposal_event.proposal.id = 7u64.try_into().expect("fits in uint48");
        input.taiko.proposal_event.proposal.proposer = Address::from([0x22; 20]);
        input.taiko.proposal_event.proposal.timestamp = 123u64.try_into().expect("fits in uint48");
        input.taiko.proposal_event.proposal.parentProposalHash = B256::from([0x33; 32]);

        let chain_spec = SupportedChainSpecs::default()
            .get_chain_spec_with_chain_id(167_000)
            .expect("supported chain");
        let mut witness = raiko2_primitives::StatelessInput {
            chain_spec,
            ..Default::default()
        };
        witness.block.header.number = 1;
        witness.block.header.timestamp = u64::MAX / 2;
        witness.block.header.parent_hash = B256::from([0x44; 32]);
        witness.block.header.state_root = B256::from([0x55; 32]);
        input.witnesses.push(witness);
        input.proof_carry_data =
            build_proof_carry_data(&input, RaikoProofType::Native).expect("build carry data");
        input
    }

    fn temp_input_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "guest-launcher-{name}-{}.json",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ))
    }

    #[test]
    fn read_input_preserves_existing_proof_carry_data() {
        let mut input = sample_guest_input();
        let expected = input.proof_carry_data.clone();
        input.proof_carry_data.transition_input.proposal_id += 9;

        let path = temp_input_path("preserve-carry");
        fs::write(&path, serde_json::to_vec(&input).expect("serialize input")).expect("write");
        let parsed = read_input(&path, ProofType::Native).expect("read input");
        fs::remove_file(path).expect("cleanup temp file");

        assert_eq!(
            parsed.proof_carry_data.transition_input.proposal_id,
            expected.transition_input.proposal_id + 9
        );
    }

    #[test]
    fn read_input_backfills_default_proof_carry_data() {
        let mut input = sample_guest_input();
        let expected = input.proof_carry_data.clone();
        input.proof_carry_data = Default::default();

        let path = temp_input_path("backfill-carry");
        fs::write(&path, serde_json::to_vec(&input).expect("serialize input")).expect("write");
        let parsed = read_input(&path, ProofType::Native).expect("read input");
        fs::remove_file(path).expect("cleanup temp file");

        assert_eq!(parsed.proof_carry_data, expected);
    }
}
