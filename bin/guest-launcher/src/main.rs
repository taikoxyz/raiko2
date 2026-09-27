pub mod controlled_workload;

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use alloy_primitives::hex;
use anyhow::{Context, Result, bail};
use clap::{Parser, ValueEnum};
use flate2::{Compression, write::GzEncoder};
use raiko2_pipeline::forks::shasta::{load_risc0_shasta_backend, load_sp1_shasta_backend};
use raiko2_pipeline::{NativeBackend, ProofStage, ProverBackend};
use raiko2_primitives::{
    AggregationGuestInput, OpcodeLabInput, PrecompileLabInput, Proof, ProofType as RaikoProofType,
};
use raiko2_primitives_shasta::GuestInput;
use raiko2_primitives_shasta::build_proof_carry_data_from_witness_spec;
use raiko2_protocol_shasta::shasta::ProofCarryData;
use raiko2_prover::Prover;
use raiko2_prover::native::NativeProver;
use raiko2_prover::sp1::{
    ProverMode as Sp1ProverMode, Sp1Config, Sp1ExecutionMetadata, Sp1FulfillmentStrategy,
    Sp1NetworkMode, Sp1Prover,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use sp1_core_executor::{
    DEFAULT_GAS_TRACE_CHUNK_SLOTS, DEFAULT_MEMORY_LIMIT, DEFAULT_TRACE_CHUNK_SLOTS,
    ELEMENT_THRESHOLD, GAS_TRACE_CHUNK_THRESHOLD, GasEstimatingVMEnum, HEIGHT_THRESHOLD,
    MINIMAL_TRACE_CHUNK_THRESHOLD, Program, SP1CoreOpts, ShardingThreshold,
};
use sp1_core_executor_runner::MinimalExecutorRunner;
use sp1_sdk::utils::setup_logger;
use sp1_sdk::{
    ExecutionReport, SP1ProvingKey, SP1Stdin,
    blocking::{Prover as BlockingProver, ProverClient as BlockingProverClient},
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
    /// SP1 execution implementation. The gas estimator is restricted to local execute-only labs
    /// and controlled production-proposal calibration blocks.
    #[arg(long, value_enum, default_value = "standard")]
    sp1_execution_engine: Sp1ExecutionEngine,
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
    #[value(name = "proposal-trace")]
    ProposalTrace,
    #[value(name = "opcode-lab")]
    OpcodeLab,
    #[value(name = "revm-opcode-lab")]
    RevmOpcodeLab,
    #[value(name = "precompile-lab")]
    PrecompileLab,
    #[value(name = "controlled-overhead")]
    ControlledOverhead,
    #[value(name = "controlled-block")]
    ControlledBlock,
    #[value(name = "controlled-state-holdout")]
    ControlledStateHoldout,
    #[value(name = "controlled-state-holdout-trace")]
    ControlledStateHoldoutTrace,
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
enum Sp1ExecutionEngine {
    Standard,
    #[value(name = "gas-estimator")]
    GasEstimator,
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
    sp1_execution_engine: &'static str,
    sp1_gas_trace_chunk_threshold: Option<u64>,
    sp1_gas_trace_chunk_slots: Option<usize>,
    input: String,
    guest_input_sha256: Option<String>,
    guest_input_bincode_length: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sp1_proposal_elf_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    guest_launcher_sha256: Option<String>,
    public_values: String,
    wall_time_ms: u64,
    primary_workload_metric: Option<BenchCountEntry>,
    workload_metrics: Vec<BenchCountEntry>,
    exit_code: Option<u64>,
    gas: Option<u64>,
    total_instruction_count: Option<u64>,
    total_syscall_count: Option<u64>,
    touched_memory_addresses: Option<u64>,
    risc0_image_id: Option<String>,
    risc0_input_bytes: Option<u64>,
    risc0_user_cycles: Option<u64>,
    risc0_padded_cycles: Option<u64>,
    risc0_segment_count: Option<u64>,
    risc0_po2_counts: Vec<BenchCountEntry>,
    cycle_tracker: Vec<BenchCycleEntry>,
    invocation_tracker: Vec<BenchCountEntry>,
    opcode_counts: Vec<BenchCountEntry>,
    syscall_counts: Vec<BenchCountEntry>,
    memory_snapshots: Vec<BenchMemoryEntry>,
    controlled_trace: Option<controlled_workload::ControlledTrace>,
    controlled_overhead: Option<ControlledOverheadRunResult>,
    controlled_block: Option<ControlledBlockRunResult>,
    controlled_state_holdout: Option<ControlledStateHoldoutRunResult>,
}

#[derive(Clone, Debug, Deserialize)]
struct ControlledOverheadRunSpec {
    target_count: u64,
    #[serde(default)]
    include_startup: bool,
}

#[derive(Clone, Debug, Serialize)]
struct ControlledOverheadRunResult {
    status: &'static str,
    target_count: u64,
    reasons: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    observation: Option<controlled_workload::ControlledOverheadObservation>,
}

#[derive(Clone, Debug, Serialize)]
struct ControlledBlockRunResult {
    status: &'static str,
    row_id: String,
    reasons: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    observation: Option<controlled_workload::ControlledBlockObservation>,
}

#[derive(Clone, Debug, Serialize)]
struct ControlledStateHoldoutRunResult {
    status: &'static str,
    pair_id: String,
    lane: controlled_workload::ControlledStateHoldoutLane,
    spec: controlled_workload::ControlledStateHoldoutPairSpec,
    reasons: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    observation: Option<controlled_workload::ControlledStateHoldoutObservation>,
}

#[derive(Clone, Debug, Serialize)]
struct ControlledStateHoldoutTraceRow {
    schema_version: u64,
    pair_id: String,
    lane: controlled_workload::ControlledStateHoldoutLane,
    spec: controlled_workload::ControlledStateHoldoutPairSpec,
    observation: controlled_workload::ControlledStateHoldoutObservation,
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
            sp1_execution_engine: Sp1ExecutionEngine::Standard.as_str(),
            sp1_gas_trace_chunk_threshold: None,
            sp1_gas_trace_chunk_slots: None,
            input,
            guest_input_sha256: None,
            guest_input_bincode_length: None,
            sp1_proposal_elf_sha256: None,
            guest_launcher_sha256: None,
            public_values: String::new(),
            wall_time_ms: 0,
            primary_workload_metric: None,
            workload_metrics: Vec::new(),
            exit_code: None,
            gas: None,
            total_instruction_count: None,
            total_syscall_count: None,
            touched_memory_addresses: None,
            risc0_image_id: None,
            risc0_input_bytes: None,
            risc0_user_cycles: None,
            risc0_padded_cycles: None,
            risc0_segment_count: None,
            risc0_po2_counts: Vec::new(),
            cycle_tracker: Vec::new(),
            invocation_tracker: Vec::new(),
            opcode_counts: Vec::new(),
            syscall_counts: Vec::new(),
            memory_snapshots: Vec::new(),
            controlled_trace: None,
            controlled_overhead: None,
            controlled_block: None,
            controlled_state_holdout: None,
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

impl ProofType {
    const fn as_raiko(self) -> RaikoProofType {
        match self {
            ProofType::Native => RaikoProofType::Native,
            ProofType::Risc0 => RaikoProofType::Risc0,
            ProofType::Sp1 => RaikoProofType::Sp1,
        }
    }
}

impl Sp1ExecutionEngine {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::GasEstimator => "gas-estimator",
        }
    }
}

fn apply_sp1_execution_engine_metadata(
    report: &mut BenchReport,
    execution_engine: Sp1ExecutionEngine,
) {
    report.sp1_execution_engine = execution_engine.as_str();
    if execution_engine == Sp1ExecutionEngine::GasEstimator {
        let opts = canonical_sp1_core_opts();
        report.sp1_gas_trace_chunk_threshold = Some(opts.gas_trace_chunk_threshold);
        report.sp1_gas_trace_chunk_slots = Some(opts.gas_trace_chunk_slots);
    }
}

impl Stage {
    const fn as_str(self) -> &'static str {
        match self {
            Stage::Proposal => "proposal",
            Stage::ProposalTrace => "proposal-trace",
            Stage::OpcodeLab => "opcode-lab",
            Stage::RevmOpcodeLab => "revm-opcode-lab",
            Stage::PrecompileLab => "precompile-lab",
            Stage::ControlledOverhead => "controlled-overhead",
            Stage::ControlledBlock => "controlled-block",
            Stage::ControlledStateHoldout => "controlled-state-holdout",
            Stage::ControlledStateHoldoutTrace => "controlled-state-holdout-trace",
        }
    }
}

impl Args {
    fn effective_sp1_prover_mode(&self) -> Sp1ProverMode {
        self.sp1_prover.map_or_else(
            || {
                if self.mode == Mode::Execute {
                    Sp1ProverMode::Local
                } else {
                    Sp1ProverMode::Network
                }
            },
            Into::into,
        )
    }

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
        let prover = self.effective_sp1_prover_mode();
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
            network_request_max_attempts: 3,
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

    fn validate_standard_guest_artifacts(&self) -> Result<()> {
        if self.stage != Stage::Proposal || self.proof_type != ProofType::Sp1 {
            return Ok(());
        }

        if self.elf.is_some() {
            bail!(
                "standard SP1 proposal and aggregation use the production guest pair; set \
                 RAIKO2_GUEST_ELF_DIR to override both artifacts"
            );
        }

        if self.mode == Mode::Prove {
            let required = if self.aggregate.is_empty() {
                ProofMode::Compressed
            } else {
                ProofMode::Plonk
            };
            if self.effective_proof_mode() != required {
                bail!(
                    "standard SP1 {} requires --proof-mode {}",
                    if self.aggregate.is_empty() {
                        "proposal"
                    } else {
                        "aggregation"
                    },
                    required.as_str()
                );
            }
        }

        Ok(())
    }

    fn validate_controlled_state_holdout_trace(&self) -> Result<()> {
        if self.stage != Stage::ControlledStateHoldoutTrace {
            bail!("controlled state holdout trace validation requires its dedicated stage");
        }
        if self.proof_type != ProofType::Native
            || self.mode != Mode::Execute
            || self.sp1_execution_engine != Sp1ExecutionEngine::Standard
        {
            bail!("controlled-state-holdout-trace supports only native execute semantics");
        }
        if self.input.is_none() {
            bail!("controlled-state-holdout-trace requires --input");
        }
        if self.jsonl_out.is_none() {
            bail!("controlled-state-holdout-trace requires --jsonl-out");
        }
        if self.input_list.is_some()
            || self.elf.is_some()
            || !self.aggregate.is_empty()
            || self.output.is_some()
            || self.json_out.is_some()
            || self.proof_mode.is_some()
            || self.sp1_prover.is_some()
            || self.sp1_network_mode != CliSp1NetworkMode::Reserved
            || self.sp1_fulfillment_strategy != CliSp1FulfillmentStrategy::Reserved
            || self.sp1_cycle_limit != 1_000_000_000_000
            || self.sp1_timeout_secs != 3_600
            || self.risc0_execution_po2 != 20
        {
            bail!("controlled-state-holdout-trace rejects prover and alternate-input flags");
        }
        Ok(())
    }

    fn validate_sp1_execution_engine(&self) -> Result<()> {
        if self.stage == Stage::ControlledStateHoldout
            && self.sp1_execution_engine != Sp1ExecutionEngine::GasEstimator
        {
            bail!("controlled-state-holdout requires --sp1-execution-engine gas-estimator");
        }
        if self.sp1_execution_engine == Sp1ExecutionEngine::Standard {
            return Ok(());
        }
        if !matches!(
            self.stage,
            Stage::Proposal
                | Stage::OpcodeLab
                | Stage::RevmOpcodeLab
                | Stage::ControlledOverhead
                | Stage::ControlledBlock
                | Stage::ControlledStateHoldout
        ) {
            bail!(
                "--sp1-execution-engine gas-estimator is restricted to proposals, opcode labs, controlled overhead, controlled blocks, and controlled state holdouts"
            );
        }
        if self.proof_type != ProofType::Sp1 {
            bail!("--sp1-execution-engine gas-estimator requires --proof-type sp1");
        }
        if self.mode != Mode::Execute {
            bail!("--sp1-execution-engine gas-estimator requires --mode execute");
        }
        if self.effective_sp1_prover_mode() != Sp1ProverMode::Local {
            bail!("--sp1-execution-engine gas-estimator requires --sp1-prover local");
        }
        if !self.aggregate.is_empty() {
            bail!("--sp1-execution-engine gas-estimator does not support --aggregate");
        }
        Ok(())
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
        Stage::Proposal
        | Stage::ProposalTrace
        | Stage::PrecompileLab
        | Stage::ControlledOverhead
        | Stage::ControlledBlock
        | Stage::ControlledStateHoldout
        | Stage::ControlledStateHoldoutTrace => {
            unreachable!("not an opcode lab stage")
        }
    }
}

fn apply_execution_metadata(report: &mut BenchReport, execution_report: &ExecutionReport) {
    let metadata =
        Sp1ExecutionMetadata::from_execution_report(report.public_values.clone(), execution_report);
    apply_sp1_metadata(report, &metadata);
}

fn install_opcode_lab_input_identity(
    report: &mut BenchReport,
    input: &OpcodeLabInput,
) -> Result<()> {
    let (sha256, bincode_length) = opcode_lab_input_identity(input)?;
    report.guest_input_sha256 = Some(sha256);
    report.guest_input_bincode_length = Some(bincode_length);
    Ok(())
}

fn finalize_opcode_lab_execution_report(
    report: &mut BenchReport,
    execution_report: &ExecutionReport,
) -> Result<()> {
    apply_execution_metadata(report, execution_report);
    match report.exit_code {
        Some(0) => Ok(()),
        Some(code) => bail!("opcode-lab guest exited with code {code}"),
        None => bail!("opcode-lab guest exit code is missing"),
    }
}

fn apply_sp1_metadata(report: &mut BenchReport, metadata: &Sp1ExecutionMetadata) {
    report.public_values = metadata.public_values.clone();
    report.exit_code = Some(metadata.exit_code);
    report.gas = metadata.gas;
    report.total_instruction_count = Some(metadata.total_instruction_count);
    report.total_syscall_count = Some(metadata.total_syscall_count);
    report.touched_memory_addresses = Some(metadata.touched_memory_addresses);
    if let Some(gas) = metadata.gas {
        report.set_primary_workload_metric("prover_gas", gas);
    }
    report.push_workload_metric(
        "sp1_total_instruction_count",
        metadata.total_instruction_count,
    );
    report.push_workload_metric("sp1_total_syscall_count", metadata.total_syscall_count);
    report.push_workload_metric(
        "sp1_touched_memory_addresses",
        metadata.touched_memory_addresses,
    );
    report.cycle_tracker = metadata
        .cycle_tracker
        .iter()
        .map(|entry| BenchCycleEntry {
            label: entry.label.clone(),
            cycles: entry.cycles,
        })
        .collect();
    report.invocation_tracker = metadata
        .invocation_tracker
        .iter()
        .map(|entry| BenchCountEntry {
            label: entry.label.clone(),
            count: entry.count,
        })
        .collect();
    report.opcode_counts = metadata
        .opcode_counts
        .iter()
        .map(|entry| BenchCountEntry {
            label: entry.label.clone(),
            count: entry.count,
        })
        .collect();
    report.syscall_counts = metadata
        .syscall_counts
        .iter()
        .map(|entry| BenchCountEntry {
            label: entry.label.clone(),
            count: entry.count,
        })
        .collect();
}

#[tokio::main]
async fn main() -> Result<()> {
    // Enable SP1 runtime logs (includes guest `println!` output).
    // Use `RUST_LOG=info` (or `debug`) when running this binary to see them.
    setup_logger();

    let args = Args::parse();
    args.validate_sp1_execution_engine()?;
    args.validate_standard_guest_artifacts()?;

    if args.stage == Stage::ProposalTrace {
        return run_proposal_trace(args);
    }
    if args.stage == Stage::ControlledOverhead {
        return run_controlled_overhead(args).await;
    }
    if args.stage == Stage::ControlledBlock {
        return run_controlled_block(args).await;
    }
    if args.stage == Stage::ControlledStateHoldoutTrace {
        return run_controlled_state_holdout_trace(args);
    }
    if args.stage == Stage::ControlledStateHoldout {
        return run_controlled_state_holdout(args).await;
    }
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

fn run_proposal_trace(args: Args) -> Result<()> {
    if args.proof_type != ProofType::Native {
        bail!("proposal-trace supports only --proof-type native");
    }
    if args.mode != Mode::Execute {
        bail!("proposal-trace supports only --mode execute");
    }
    if !args.aggregate.is_empty() {
        bail!("proposal-trace does not support --aggregate proofs");
    }
    if args.input_list.is_some() {
        bail!("proposal-trace does not support --input-list");
    }
    if args.elf.is_some() {
        bail!("proposal-trace does not support --elf");
    }
    if args.output.is_some() {
        bail!("proposal-trace does not produce --output proof files");
    }
    if args.jsonl_out.is_some() {
        bail!("proposal-trace does not support --jsonl-out");
    }

    let input_path = args.input.as_ref().context("missing --input")?;
    let input = read_input(input_path, ProofType::Sp1)?;
    let trace = raiko2_zkgas_trace::trace_shasta_proposal(&input)?;
    if let Some(path) = &args.json_out {
        write_proposal_trace_outputs(path, &trace)?;
    } else {
        let stdout = io::stdout();
        let mut output = stdout.lock();
        serde_json::to_writer(&mut output, &trace).context("serialize proposal trace")?;
        output.write_all(b"\n").context("write proposal trace")?;
    }
    Ok(())
}

fn proposal_trace_summary_path(trace_path: &Path) -> PathBuf {
    let name = trace_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("proposal-trace");
    let stem = name.strip_suffix(".json.gz").unwrap_or(name);
    trace_path.with_file_name(format!("{stem}.summary.json"))
}

fn write_proposal_trace_outputs(
    trace_path: &Path,
    trace: &raiko2_zkgas_trace::ProposalTrace,
) -> Result<()> {
    let trace_file =
        fs::File::create(trace_path).with_context(|| format!("create {}", trace_path.display()))?;
    let mut encoder = GzEncoder::new(BufWriter::new(trace_file), Compression::default());
    serde_json::to_writer(&mut encoder, trace).context("serialize compressed proposal trace")?;
    let mut trace_output = encoder
        .finish()
        .context("finish compressed proposal trace")?;
    trace_output
        .flush()
        .context("flush compressed proposal trace")?;

    let summary_path = proposal_trace_summary_path(trace_path);
    let summary_file = fs::File::create(&summary_path)
        .with_context(|| format!("create {}", summary_path.display()))?;
    let mut summary_output = BufWriter::new(summary_file);
    serde_json::to_writer(&mut summary_output, &trace.summary())
        .context("serialize proposal trace summary")?;
    summary_output
        .write_all(b"\n")
        .context("write proposal trace summary")?;
    summary_output
        .flush()
        .context("flush proposal trace summary")?;
    Ok(())
}

fn read_input(path: &PathBuf, proof_type: ProofType) -> Result<GuestInput> {
    let contents = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let mut input: GuestInput = serde_json::from_str(&contents).context("parse input JSON")?;
    if !input.witnesses.is_empty() && input.proof_carry_data == ProofCarryData::default() {
        input.proof_carry_data =
            build_proof_carry_data_from_witness_spec(&input, proof_type.as_raiko())?;
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
    apply_sp1_execution_engine_metadata(&mut report, args.sp1_execution_engine);
    let labels = opcode_lab_memory_labels(args.stage);
    record_memory_snapshot(&mut report, labels.start);

    let input = read_opcode_lab_input(&input_path)?;
    install_opcode_lab_input_identity(&mut report, &input)?;
    apply_controlled_opcode_trace(&mut report, args.stage, &input)?;
    record_memory_snapshot(&mut report, labels.after_read_input);
    let mut stdin = SP1Stdin::new();
    stdin.write(&input);
    record_memory_snapshot(&mut report, labels.after_stdin_write);
    let elf = fs::read(&elf_path).with_context(|| format!("read {}", elf_path.display()))?;
    record_memory_snapshot(&mut report, labels.after_load_elf);

    let sp1_config = args.sp1_config()?;
    let start = Instant::now();
    record_memory_snapshot(&mut report, labels.before_execute_run);
    let (public_values, execution_report) = match args.sp1_execution_engine {
        Sp1ExecutionEngine::Standard => execute_sp1_blocking(sp1_config.prover, elf, stdin).await?,
        Sp1ExecutionEngine::GasEstimator => {
            execute_opcode_lab_gas_estimator_blocking(elf, input).await?
        }
    };
    record_memory_snapshot(&mut report, labels.after_execute_run);
    report.wall_time_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
    report.public_values = public_values.raw();
    finalize_opcode_lab_execution_report(&mut report, &execution_report)?;
    record_memory_snapshot(&mut report, labels.after_apply_execution_metadata);

    println!("public_values: {}", report.public_values);
    if !execution_report.cycle_tracker.is_empty() {
        println!("cycle_tracker:");
        for (label, cycles) in &execution_report.cycle_tracker {
            println!("  {label}: {cycles}");
        }
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
    let runs = execute_opcode_lab_batch_blocking(
        sp1_config.prover,
        args.sp1_execution_engine,
        elf,
        inputs,
        args.stage,
    )
    .await?;

    let mut output = String::new();
    for run in runs {
        let mut report = BenchReport::new(
            args.stage.as_str(),
            args.mode.as_str(),
            proof_mode.as_str(),
            run.input_path.display().to_string(),
        );
        apply_sp1_execution_engine_metadata(&mut report, args.sp1_execution_engine);
        report.public_values = run.public_values;
        report.wall_time_ms = run.wall_time_ms;
        report.guest_input_sha256 = run.guest_input_sha256;
        report.guest_input_bincode_length = run.guest_input_bincode_length;
        if let Some(trace) = run.controlled_trace {
            install_controlled_trace(&mut report, trace)?;
        }
        finalize_opcode_lab_execution_report(&mut report, &run.execution_report)?;
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
    apply_controlled_precompile_trace(&mut report, &input)?;
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
        if let Some(trace) = run.controlled_trace {
            install_controlled_trace(&mut report, trace)?;
        }
        apply_execution_metadata(&mut report, &run.execution_report);
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
    let (guest_input_sha256, guest_input_bincode_length) =
        raiko2_zkgas_trace::guest_input_identity(&input)?;
    report.guest_input_sha256 = Some(guest_input_sha256);
    report.guest_input_bincode_length = Some(guest_input_bincode_length);
    record_memory_snapshot(&mut report, "proposal:after_read_input");

    match args.proof_type {
        ProofType::Sp1 => run_sp1_proposal(args, input_path, input, report).await,
        ProofType::Native => run_native_proposal(args, input_path, input, report).await,
        ProofType::Risc0 => run_risc0_proposal(args, input_path, input, report).await,
    }
}

fn new_controlled_overhead_report(
    execution_engine: Sp1ExecutionEngine,
    input: String,
) -> BenchReport {
    let mut report = BenchReport::new("controlled-overhead", "execute", "compressed", input);
    apply_sp1_execution_engine_metadata(&mut report, execution_engine);
    report
}

async fn run_controlled_overhead(args: Args) -> Result<()> {
    if args.proof_type != ProofType::Sp1 || args.mode != Mode::Execute {
        bail!("controlled-overhead supports only SP1 execute mode");
    }
    if args.elf.is_some() || args.input_list.is_some() || !args.aggregate.is_empty() {
        bail!("controlled-overhead always uses the production SP1 proposal guest");
    }
    if args.output.is_some() || args.json_out.is_some() {
        bail!("controlled-overhead writes only --jsonl-out benchmark rows");
    }
    let input_path = args.input.as_ref().context("missing --input")?;
    let output_path = args
        .jsonl_out
        .as_ref()
        .context("controlled-overhead requires --jsonl-out")?;
    let spec: ControlledOverheadRunSpec = serde_json::from_slice(
        &fs::read(input_path).with_context(|| format!("read {}", input_path.display()))?,
    )
    .context("parse controlled-overhead input")?;
    if spec.target_count.saturating_add(1) > 768 {
        let mut report = new_controlled_overhead_report(
            args.sp1_execution_engine,
            input_path.display().to_string(),
        );
        report.controlled_overhead = Some(ControlledOverheadRunResult {
            status: "rejected",
            target_count: spec.target_count,
            reasons: vec!["generation_failure".into(), "protocol_block_bound".into()],
            error: Some("block_base target requires more than 768 source blocks".into()),
            observation: None,
        });
        fs::write(output_path, serde_json::to_string(&report)? + "\n")
            .with_context(|| format!("write {}", output_path.display()))?;
        return Ok(());
    }
    let mut fixtures = controlled_workload::build_required_overhead_fixtures(spec.target_count)?;
    if !spec.include_startup {
        fixtures.retain(|fixture| fixture.overhead_key_id != "proposal_startup");
    }
    let observations = controlled_workload::validate_required_overhead_fixtures(&fixtures)?;
    let backend = load_sp1_shasta_backend()
        .map_err(anyhow::Error::msg)
        .context("load production SP1 Shasta guest ELFs")?;
    let standard_prover = match args.sp1_execution_engine {
        Sp1ExecutionEngine::Standard => Some(Sp1Prover::new(args.sp1_config()?)),
        Sp1ExecutionEngine::GasEstimator => None,
    };
    let estimator_elf = match args.sp1_execution_engine {
        Sp1ExecutionEngine::Standard => None,
        Sp1ExecutionEngine::GasEstimator => Some(
            backend
                .elf(ProofStage::Proposal)
                .map_err(anyhow::Error::msg)
                .context("load production SP1 proposal ELF")?
                .to_vec(),
        ),
    };
    let mut output = String::new();
    for (fixture, observation) in fixtures.into_iter().zip(observations) {
        let lane = serde_json::to_value(fixture.lane)?
            .as_str()
            .unwrap_or("unknown")
            .to_string();
        let mut report = new_controlled_overhead_report(
            args.sp1_execution_engine,
            format!("{}:{lane}:{}", fixture.case_id, fixture.target_count),
        );
        report.guest_input_sha256 = Some(observation.guest_input_sha256.clone());
        report.guest_input_bincode_length = Some(observation.guest_input_bincode_length);
        record_memory_snapshot(&mut report, "controlled-overhead:before_sp1_prover");
        let start = Instant::now();
        match args.sp1_execution_engine {
            Sp1ExecutionEngine::Standard => {
                let proof = standard_prover
                    .as_ref()
                    .expect("standard engine initializes the SP1 prover")
                    .prove(fixture.guest_input, &serde_json::Value::Null, &backend)
                    .await
                    .with_context(|| {
                        format!(
                            "production SP1 proposal failed for {} {lane}",
                            fixture.case_id
                        )
                    })?;
                let metadata_value = proof
                    .extra_data
                    .as_ref()
                    .and_then(|extra_data| extra_data.get("sp1"))
                    .cloned()
                    .context("controlled-overhead SP1 execute is missing production metadata")?;
                let metadata: Sp1ExecutionMetadata = serde_json::from_value(metadata_value)
                    .context("parse controlled-overhead SP1 execution metadata")?;
                apply_sp1_metadata(&mut report, &metadata);
            }
            Sp1ExecutionEngine::GasEstimator => {
                let (public_values, execution_report) = execute_sp1_guest_gas_estimator_blocking(
                    estimator_elf
                        .as_ref()
                        .expect("gas-estimator engine loads the production proposal ELF")
                        .clone(),
                    fixture.guest_input,
                )
                .await?;
                report.public_values = public_values.raw();
                apply_execution_metadata(&mut report, &execution_report);
            }
        }
        report.wall_time_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
        if report.public_values != format!("{:#x}", observation.public_output).to_lowercase() {
            bail!(
                "controlled-overhead trace/SP1 public output mismatch for {} {lane}",
                fixture.case_id
            );
        }
        report.controlled_overhead = Some(ControlledOverheadRunResult {
            status: "accepted",
            target_count: spec.target_count,
            reasons: Vec::new(),
            error: None,
            observation: Some(observation),
        });
        output.push_str(&serde_json::to_string(&report)?);
        output.push('\n');
    }
    fs::write(output_path, output).with_context(|| format!("write {}", output_path.display()))?;
    Ok(())
}

async fn run_controlled_block(args: Args) -> Result<()> {
    if args.proof_type != ProofType::Sp1 || args.mode != Mode::Execute {
        bail!("controlled-block supports only SP1 execute mode");
    }
    if args.elf.is_some() || args.input_list.is_some() || !args.aggregate.is_empty() {
        bail!("controlled-block always uses the production SP1 proposal guest");
    }
    if args.output.is_some() || args.json_out.is_some() {
        bail!("controlled-block writes only --jsonl-out benchmark rows");
    }
    let input_path = args.input.as_ref().context("missing --input")?;
    let output_path = args
        .jsonl_out
        .as_ref()
        .context("controlled-block requires --jsonl-out")?;
    let spec: controlled_workload::ControlledBlockRowSpec = serde_json::from_slice(
        &fs::read(input_path).with_context(|| format!("read {}", input_path.display()))?,
    )
    .context("parse controlled-block input")?;
    let mut report = BenchReport::new(
        "controlled-block",
        "execute",
        "compressed",
        spec.row_id.clone(),
    );
    apply_sp1_execution_engine_metadata(&mut report, args.sp1_execution_engine);
    let fixture = match controlled_workload::build_controlled_block_fixture(&spec) {
        Ok(fixture) => fixture,
        Err(error) => {
            report.controlled_block = Some(ControlledBlockRunResult {
                status: "rejected",
                row_id: spec.row_id,
                reasons: vec!["generation_failure".into()],
                error: Some(format!("{error:#}")),
                observation: None,
            });
            fs::write(output_path, serde_json::to_string(&report)? + "\n")
                .with_context(|| format!("write {}", output_path.display()))?;
            return Ok(());
        }
    };
    let observation = match controlled_workload::validate_controlled_block_fixture(&fixture) {
        Ok(observation) => observation,
        Err(error) => {
            report.controlled_block = Some(ControlledBlockRunResult {
                status: "rejected",
                row_id: spec.row_id,
                reasons: vec!["host_trace_mismatch".into()],
                error: Some(format!("{error:#}")),
                observation: None,
            });
            fs::write(output_path, serde_json::to_string(&report)? + "\n")
                .with_context(|| format!("write {}", output_path.display()))?;
            return Ok(());
        }
    };
    report.guest_input_sha256 = Some(format!("0x{}", observation.backend_input_sha256));
    report.guest_input_bincode_length = Some(observation.guest_input_bincode_length);
    let backend = load_sp1_shasta_backend()
        .map_err(anyhow::Error::msg)
        .context("load production SP1 Shasta guest ELFs")?;
    record_memory_snapshot(&mut report, "controlled-block:before_sp1_prover");
    let start = Instant::now();
    match args.sp1_execution_engine {
        Sp1ExecutionEngine::Standard => {
            let prover = Sp1Prover::new(args.sp1_config()?);
            let proof = prover
                .prove(fixture.guest_input, &serde_json::Value::Null, &backend)
                .await
                .with_context(|| format!("production SP1 proposal failed for {}", spec.row_id))?;
            let metadata_value = proof
                .extra_data
                .as_ref()
                .and_then(|extra_data| extra_data.get("sp1"))
                .cloned()
                .context("controlled-block SP1 execute is missing production metadata")?;
            let metadata: Sp1ExecutionMetadata = serde_json::from_value(metadata_value)
                .context("parse controlled-block SP1 execution metadata")?;
            apply_sp1_metadata(&mut report, &metadata);
        }
        Sp1ExecutionEngine::GasEstimator => {
            let elf = backend
                .elf(ProofStage::Proposal)
                .map_err(anyhow::Error::msg)
                .context("load production SP1 proposal ELF")?
                .to_vec();
            let (public_values, execution_report) =
                execute_sp1_guest_gas_estimator_blocking(elf, fixture.guest_input).await?;
            report.public_values = public_values.raw();
            apply_execution_metadata(&mut report, &execution_report);
        }
    }
    report.wall_time_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
    if report.public_values != format!("{:#x}", observation.public_output).to_lowercase() {
        bail!(
            "controlled-block trace/SP1 public output mismatch for {}",
            spec.row_id
        );
    }
    report.controlled_block = Some(ControlledBlockRunResult {
        status: "accepted",
        row_id: spec.row_id,
        reasons: Vec::new(),
        error: None,
        observation: Some(observation),
    });
    fs::write(output_path, serde_json::to_string(&report)? + "\n")
        .with_context(|| format!("write {}", output_path.display()))?;
    Ok(())
}

fn prepare_controlled_state_holdout(
    input_path: &Path,
) -> Result<(
    controlled_workload::ControlledStateHoldoutPairSpec,
    Vec<controlled_workload::ControlledStateHoldoutFixture>,
    Vec<controlled_workload::ControlledStateHoldoutObservation>,
)> {
    let spec: controlled_workload::ControlledStateHoldoutPairSpec = serde_json::from_slice(
        &fs::read(input_path).with_context(|| format!("read {}", input_path.display()))?,
    )
    .context("parse controlled state holdout input")?;
    let fixtures = controlled_workload::build_controlled_state_holdout_fixtures(&spec)?;
    let observations = controlled_workload::validate_controlled_state_holdout_fixtures(&fixtures)?;
    Ok((spec, fixtures, observations))
}

fn run_controlled_state_holdout_trace(args: Args) -> Result<()> {
    args.validate_controlled_state_holdout_trace()?;
    let input_path = args
        .input
        .as_ref()
        .context("controlled-state-holdout-trace requires --input")?;
    let output_path = args
        .jsonl_out
        .as_ref()
        .context("controlled-state-holdout-trace requires --jsonl-out")?;
    let (spec, fixtures, observations) = prepare_controlled_state_holdout(input_path)?;
    let mut output = String::new();
    for (fixture, observation) in fixtures.into_iter().zip(observations) {
        let row = ControlledStateHoldoutTraceRow {
            schema_version: 1,
            pair_id: fixture.pair_id,
            lane: fixture.lane,
            spec: spec.clone(),
            observation,
        };
        output.push_str(&serde_json::to_string(&row)?);
        output.push('\n');
    }
    fs::write(output_path, output).with_context(|| format!("write {}", output_path.display()))?;
    Ok(())
}

async fn run_controlled_state_holdout(args: Args) -> Result<()> {
    if args.proof_type != ProofType::Sp1
        || args.mode != Mode::Execute
        || args.effective_sp1_prover_mode() != Sp1ProverMode::Local
        || args.sp1_execution_engine != Sp1ExecutionEngine::GasEstimator
    {
        bail!(
            "controlled-state-holdout supports only local SP1 execute mode with the gas-estimator engine"
        );
    }
    if args.elf.is_some() || args.input_list.is_some() || !args.aggregate.is_empty() {
        bail!("controlled-state-holdout always uses the production SP1 proposal guest");
    }
    if args.output.is_some() || args.json_out.is_some() {
        bail!("controlled-state-holdout writes only --jsonl-out benchmark rows");
    }
    let input_path = args.input.as_ref().context("missing --input")?;
    let output_path = args
        .jsonl_out
        .as_ref()
        .context("controlled-state-holdout requires --jsonl-out")?;
    let (_spec, fixtures, observations) = prepare_controlled_state_holdout(input_path)?;
    let backend = load_sp1_shasta_backend()
        .map_err(anyhow::Error::msg)
        .context("load production SP1 Shasta guest ELFs")?;
    let elf = backend
        .elf(ProofStage::Proposal)
        .map_err(anyhow::Error::msg)
        .context("load production SP1 proposal ELF")?
        .to_vec();
    let mut output = String::new();
    for (fixture, observation) in fixtures.into_iter().zip(observations) {
        let lane = serde_json::to_value(fixture.lane)?
            .as_str()
            .unwrap_or("unknown")
            .to_string();
        let mut report = BenchReport::new(
            "controlled-state-holdout",
            "execute",
            "compressed",
            format!("{}:{lane}", fixture.pair_id),
        );
        apply_sp1_execution_engine_metadata(&mut report, args.sp1_execution_engine);
        report.guest_input_sha256 = Some(observation.guest_input_sha256.clone());
        report.guest_input_bincode_length = Some(observation.guest_input_bincode_length);
        record_memory_snapshot(&mut report, "controlled-state-holdout:before_sp1_prover");
        let start = Instant::now();
        let (public_values, execution_report) =
            execute_sp1_guest_gas_estimator_blocking(elf.clone(), fixture.guest_input).await?;
        report.wall_time_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
        report.public_values = public_values.raw();
        apply_execution_metadata(&mut report, &execution_report);
        if report.public_values != format!("{:#x}", observation.public_output).to_lowercase() {
            bail!(
                "controlled-state-holdout trace/SP1 public output mismatch for {} {lane}",
                fixture.pair_id
            );
        }
        report.controlled_state_holdout = Some(ControlledStateHoldoutRunResult {
            status: "accepted",
            pair_id: fixture.pair_id,
            lane: fixture.lane,
            spec: fixture.spec,
            reasons: Vec::new(),
            error: None,
            observation: Some(observation),
        });
        output.push_str(&serde_json::to_string(&report)?);
        output.push('\n');
    }
    fs::write(output_path, output).with_context(|| format!("write {}", output_path.display()))?;
    Ok(())
}

async fn run_aggregation(args: Args) -> Result<()> {
    if args.proof_type != ProofType::Sp1 {
        anyhow::bail!("aggregation is supported only for --proof-type sp1");
    }
    if args.aggregate.is_empty() {
        anyhow::bail!("missing --aggregate proofs");
    }
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

    let sp1_config = args.sp1_config()?;
    let backend = load_sp1_shasta_backend()
        .map_err(anyhow::Error::msg)
        .context("load SP1 Shasta guest ELFs")?;
    let prover = Sp1Prover::new(sp1_config);
    let proofs = read_proofs(&args.aggregate)?;
    let start = Instant::now();
    let proof = prover
        .aggregate(
            AggregationGuestInput { proofs },
            &serde_json::Value::Null,
            &backend,
        )
        .await
        .context("SP1 aggregation failed")?;
    let mut report = BenchReport::new(
        "aggregation",
        args.mode.as_str(),
        proof_mode.as_str(),
        output_path.display().to_string(),
    );
    report.public_values = proof
        .input
        .map(|input| format!("{input:#x}"))
        .unwrap_or_default();
    report.wall_time_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);

    write_proof_json(output_path, &proof)?;

    println!("public_values: {}", report.public_values);

    if let Some(path) = &args.json_out {
        let contents = serde_json::to_string_pretty(&report).context("serialize bench report")?;
        fs::write(path, contents).with_context(|| format!("write {}", path.display()))?;
    }

    Ok(())
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
    guest_input_sha256: Option<String>,
    guest_input_bincode_length: Option<usize>,
    controlled_trace: Option<controlled_workload::ControlledTrace>,
}

fn opcode_lab_input_identity(input: &OpcodeLabInput) -> Result<(String, usize)> {
    let encoded = bincode::serialize(input).context("serialize canonical opcode-lab input")?;
    Ok((
        format!("0x{}", hex::encode(Sha256::digest(&encoded))),
        encoded.len(),
    ))
}

fn install_controlled_trace(
    report: &mut BenchReport,
    trace: controlled_workload::ControlledTrace,
) -> Result<()> {
    let (backend_input_sha256, backend_input_len) = match &trace {
        controlled_workload::ControlledTrace::RevmOpcode(trace) => {
            (&trace.backend_input_sha256, trace.backend_input_len)
        }
        controlled_workload::ControlledTrace::Precompile(trace) => {
            (&trace.backend_input_sha256, trace.backend_input_len)
        }
    };
    let trace_sha256 = format!("0x{backend_input_sha256}");
    if report
        .guest_input_sha256
        .as_ref()
        .is_some_and(|value| value != &trace_sha256)
        || report
            .guest_input_bincode_length
            .is_some_and(|value| value != backend_input_len)
    {
        bail!("controlled trace input identity differs from canonical guest input");
    }
    report.guest_input_sha256 = Some(trace_sha256);
    report.guest_input_bincode_length = Some(backend_input_len);
    report.controlled_trace = Some(trace);
    Ok(())
}

fn apply_controlled_opcode_trace(
    report: &mut BenchReport,
    stage: Stage,
    input: &OpcodeLabInput,
) -> Result<()> {
    if stage == Stage::RevmOpcodeLab {
        install_controlled_trace(
            report,
            controlled_workload::ControlledTrace::RevmOpcode(
                controlled_workload::trace_revm_opcode_workload(input)?,
            ),
        )?;
    }
    Ok(())
}

fn apply_controlled_precompile_trace(
    report: &mut BenchReport,
    input: &PrecompileLabInput,
) -> Result<()> {
    install_controlled_trace(
        report,
        controlled_workload::ControlledTrace::Precompile(
            controlled_workload::trace_precompile_workload(input)?,
        ),
    )
}

struct Risc0ProposalExecution {
    public_values: String,
    wall_time_ms: u64,
    image_id: String,
    input_bytes: u64,
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
    report.risc0_image_id = Some(execution.image_id.clone());
    report.risc0_input_bytes = Some(execution.input_bytes);
    report.risc0_user_cycles = Some(execution.user_cycles);
    report.risc0_padded_cycles = Some(execution.padded_cycles);
    report.risc0_segment_count = Some(execution.segment_count);
    report.risc0_po2_counts = execution.po2_counts.clone();
    report.set_primary_workload_metric("risc0_padded_cycles", execution.padded_cycles);
    report.push_workload_metric("risc0_input_bytes", execution.input_bytes);
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
        let input_bytes = u64::try_from(encoded.len()).unwrap_or(u64::MAX);
        let image_id =
            risc0_zkvm::compute_image_id(&elf).context("compute RISC0 proposal image ID")?;
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
            image_id: hex::encode_prefixed(image_id.as_bytes()),
            input_bytes,
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
    execution_engine: Sp1ExecutionEngine,
    elf: Vec<u8>,
    inputs: Vec<(PathBuf, OpcodeLabInput)>,
    stage: Stage,
) -> Result<Vec<OpcodeLabExecution>> {
    tokio::task::spawn_blocking(move || match execution_engine {
        Sp1ExecutionEngine::Standard => match prover_mode {
            Sp1ProverMode::Mock => {
                let prover = BlockingProverClient::builder().mock().build();
                execute_opcode_lab_batch_local(&prover, &elf, inputs, stage)
            }
            Sp1ProverMode::Local => {
                let prover = BlockingProverClient::builder().cpu().build();
                execute_opcode_lab_batch_local(&prover, &elf, inputs, stage)
            }
            Sp1ProverMode::Network => {
                anyhow::bail!("sp1.mode=execute does not support sp1.prover=network")
            }
        },
        Sp1ExecutionEngine::GasEstimator => {
            if prover_mode != Sp1ProverMode::Local {
                anyhow::bail!("gas-estimator execution requires the local SP1 prover")
            }
            execute_opcode_lab_batch_gas_estimator(&elf, inputs, stage)
        }
    })
    .await
    .context("join SP1 blocking opcode-lab batch task")?
}

async fn execute_opcode_lab_gas_estimator_blocking(
    elf: Vec<u8>,
    input: OpcodeLabInput,
) -> Result<(sp1_sdk::SP1PublicValues, ExecutionReport)> {
    tokio::task::spawn_blocking(move || {
        let program = parse_sp1_program(&elf)?;
        execute_opcode_lab_gas_estimator(program, &input)
    })
    .await
    .context("join SP1 gas-estimator opcode-lab task")?
}

fn parse_sp1_program(elf: &[u8]) -> Result<Arc<Program>> {
    Program::from(elf)
        .map(Arc::new)
        .map_err(|err| anyhow::anyhow!("parse SP1 guest ELF: {err:?}"))
}

fn execute_opcode_lab_gas_estimator(
    program: Arc<Program>,
    input: &OpcodeLabInput,
) -> Result<(sp1_sdk::SP1PublicValues, ExecutionReport)> {
    execute_opcode_lab_gas_estimator_with_opts(program, input, canonical_sp1_core_opts())
}

async fn execute_sp1_guest_gas_estimator_blocking(
    elf: Vec<u8>,
    guest_input: GuestInput,
) -> Result<(sp1_sdk::SP1PublicValues, ExecutionReport)> {
    tokio::task::spawn_blocking(move || {
        let program = parse_sp1_program(&elf)?;
        let mut stdin = SP1Stdin::new();
        stdin.write(&guest_input);
        execute_sp1_gas_estimator(program, stdin)
    })
    .await
    .context("join SP1 guest gas-estimator task")?
}

fn canonical_sp1_core_opts() -> SP1CoreOpts {
    canonicalize_sp1_core_opts(SP1CoreOpts::default())
}

fn canonicalize_sp1_core_opts(mut opts: SP1CoreOpts) -> SP1CoreOpts {
    opts.minimal_trace_chunk_threshold = MINIMAL_TRACE_CHUNK_THRESHOLD;
    opts.gas_trace_chunk_threshold = GAS_TRACE_CHUNK_THRESHOLD;
    opts.trace_chunk_slots = DEFAULT_TRACE_CHUNK_SLOTS;
    opts.gas_trace_chunk_slots = DEFAULT_GAS_TRACE_CHUNK_SLOTS;
    opts.memory_limit = DEFAULT_MEMORY_LIMIT;
    opts.shard_size = 1 << 24;
    opts.sharding_threshold = ShardingThreshold {
        element_threshold: ELEMENT_THRESHOLD,
        height_threshold: HEIGHT_THRESHOLD,
    };
    opts
}

fn execute_opcode_lab_gas_estimator_with_opts(
    program: Arc<Program>,
    input: &OpcodeLabInput,
    opts: SP1CoreOpts,
) -> Result<(sp1_sdk::SP1PublicValues, ExecutionReport)> {
    let mut stdin = SP1Stdin::new();
    stdin.write(input);
    execute_sp1_gas_estimator_with_opts(program, stdin, opts)
}

fn execute_sp1_gas_estimator(
    program: Arc<Program>,
    stdin: SP1Stdin,
) -> Result<(sp1_sdk::SP1PublicValues, ExecutionReport)> {
    execute_sp1_gas_estimator_with_opts(program, stdin, canonical_sp1_core_opts())
}

fn execute_sp1_gas_estimator_with_opts(
    program: Arc<Program>,
    stdin: SP1Stdin,
    opts: SP1CoreOpts,
) -> Result<(sp1_sdk::SP1PublicValues, ExecutionReport)> {
    let mut runner = MinimalExecutorRunner::new(
        program.clone(),
        false,
        Some(opts.gas_trace_chunk_threshold),
        opts.memory_limit,
        opts.gas_trace_chunk_slots,
    );
    for buffer in &stdin.buffer {
        runner.with_input(buffer);
    }

    let mut report = ExecutionReport::default();
    while let Some(chunk) = runner
        .try_execute_chunk()
        .map_err(|err| anyhow::anyhow!("execute minimal SP1 guest chunk: {err:?}"))?
    {
        let mut vm = GasEstimatingVMEnum::new(&chunk, program.clone(), [0u32; 4], opts.clone());
        report += vm
            .execute()
            .map_err(|err| anyhow::anyhow!("estimate SP1 guest gas: {err:?}"))?;
    }
    let public_values = sp1_sdk::SP1PublicValues::from(runner.public_values_stream());
    Ok((public_values, report))
}

fn execute_opcode_lab_batch_gas_estimator(
    elf: &[u8],
    inputs: Vec<(PathBuf, OpcodeLabInput)>,
    stage: Stage,
) -> Result<Vec<OpcodeLabExecution>> {
    let program = parse_sp1_program(elf)?;
    let mut outputs = Vec::with_capacity(inputs.len());
    for (input_path, input) in inputs {
        let (guest_input_sha256, guest_input_bincode_length) = opcode_lab_input_identity(&input)?;
        let controlled_trace = if stage == Stage::RevmOpcodeLab {
            Some(controlled_workload::ControlledTrace::RevmOpcode(
                controlled_workload::trace_revm_opcode_workload(&input)?,
            ))
        } else {
            None
        };
        let start = Instant::now();
        let (public_values, execution_report) =
            execute_opcode_lab_gas_estimator(program.clone(), &input)?;
        outputs.push(OpcodeLabExecution {
            input_path,
            public_values: public_values.raw(),
            wall_time_ms: u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX),
            execution_report,
            guest_input_sha256: Some(guest_input_sha256),
            guest_input_bincode_length: Some(guest_input_bincode_length),
            controlled_trace,
        });
    }
    Ok(outputs)
}

fn execute_opcode_lab_batch_local<P>(
    prover: &P,
    elf: &[u8],
    inputs: Vec<(PathBuf, OpcodeLabInput)>,
    stage: Stage,
) -> Result<Vec<OpcodeLabExecution>>
where
    P: BlockingProver<ProvingKey = SP1ProvingKey>,
{
    let mut outputs = Vec::with_capacity(inputs.len());
    for (input_path, input) in inputs {
        let (guest_input_sha256, guest_input_bincode_length) = opcode_lab_input_identity(&input)?;
        let controlled_trace = if stage == Stage::RevmOpcodeLab {
            Some(controlled_workload::ControlledTrace::RevmOpcode(
                controlled_workload::trace_revm_opcode_workload(&input)?,
            ))
        } else {
            None
        };
        let mut stdin = SP1Stdin::new();
        stdin.write(&input);
        let start = Instant::now();
        let (public_values, execution_report) = execute_sp1_local(prover, elf, stdin)?;
        outputs.push(OpcodeLabExecution {
            input_path,
            public_values: public_values.raw(),
            wall_time_ms: u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX),
            execution_report,
            guest_input_sha256: Some(guest_input_sha256),
            guest_input_bincode_length: Some(guest_input_bincode_length),
            controlled_trace,
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
        let controlled_trace = Some(controlled_workload::ControlledTrace::Precompile(
            controlled_workload::trace_precompile_workload(&input)?,
        ));
        let mut stdin = SP1Stdin::new();
        stdin.write(&input);
        let start = Instant::now();
        let (public_values, execution_report) = execute_sp1_local(prover, elf, stdin)?;
        outputs.push(OpcodeLabExecution {
            input_path,
            public_values: public_values.raw(),
            wall_time_ms: u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX),
            execution_report,
            guest_input_sha256: None,
            guest_input_bincode_length: None,
            controlled_trace,
        });
    }
    Ok(outputs)
}

async fn run_sp1_proposal(
    args: Args,
    input_path: PathBuf,
    input: GuestInput,
    mut report: BenchReport,
) -> Result<()> {
    if args.sp1_execution_engine == Sp1ExecutionEngine::GasEstimator {
        validate_proposal_gas_estimator_guest_elf_override(
            std::env::var_os("RAIKO2_GUEST_ELF_DIR").as_deref(),
        )?;
    }
    let backend = load_sp1_shasta_backend()
        .map_err(anyhow::Error::msg)
        .context("load SP1 Shasta guest ELFs")?;
    report.input = input_path.display().to_string();
    record_memory_snapshot(&mut report, "proposal:before_sp1_prover");
    let start = Instant::now();
    match args.sp1_execution_engine {
        Sp1ExecutionEngine::Standard => {
            let prover = Sp1Prover::new(args.sp1_config()?);
            let proof = prover
                .prove(input, &serde_json::Value::Null, &backend)
                .await
                .context("SP1 proposal failed")?;
            if args.mode == Mode::Execute {
                let metadata_value = proof
                    .extra_data
                    .as_ref()
                    .and_then(|extra_data| extra_data.get("sp1"))
                    .cloned()
                    .context("SP1 execute proof is missing production metadata")?;
                let metadata: Sp1ExecutionMetadata = serde_json::from_value(metadata_value)
                    .context("parse production SP1 execution metadata")?;
                apply_sp1_metadata(&mut report, &metadata);
            } else {
                report.public_values = proof
                    .input
                    .map(|input| format!("{input:#x}"))
                    .unwrap_or_default();
                if let Some(path) = &args.output {
                    write_proof_json(path, &proof)?;
                }
            }
        }
        Sp1ExecutionEngine::GasEstimator => {
            let elf = backend
                .elf(ProofStage::Proposal)
                .map_err(anyhow::Error::msg)
                .context("load production SP1 proposal ELF")?
                .to_vec();
            let proposal_elf_sha256 = hex::encode(Sha256::digest(&elf));
            let guest_launcher_sha256 = current_guest_launcher_sha256()?;
            let (public_values, execution_report) =
                execute_sp1_guest_gas_estimator_blocking(elf, input).await?;
            finalize_sp1_proposal_gas_estimator_report(
                &mut report,
                public_values.raw(),
                &execution_report,
                proposal_elf_sha256,
                guest_launcher_sha256,
            )?;
        }
    }
    report.wall_time_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
    record_memory_snapshot(&mut report, "proposal:after_sp1_prover");

    if args.mode == Mode::Execute {
        record_memory_snapshot(&mut report, "proposal:after_apply_execution_metadata");
        if !report.cycle_tracker.is_empty() {
            println!("cycle_tracker:");
            for entry in &report.cycle_tracker {
                println!("  {}: {}", entry.label, entry.cycles);
            }
        }
    }

    println!("public_values: {}", report.public_values);

    if let Some(path) = &args.json_out {
        let contents = serde_json::to_string_pretty(&report).context("serialize bench report")?;
        fs::write(path, contents).with_context(|| format!("write {}", path.display()))?;
    }

    Ok(())
}

fn validate_proposal_gas_estimator_guest_elf_override(
    guest_elf_dir: Option<&std::ffi::OsStr>,
) -> Result<()> {
    if guest_elf_dir.is_some() {
        bail!(
            "proposal gas-estimator requires the production SP1 proposal ELF and rejects \
             RAIKO2_GUEST_ELF_DIR"
        );
    }
    Ok(())
}

fn current_guest_launcher_sha256() -> Result<String> {
    let path = guest_launcher_executable_path()?;
    let bytes = fs::read(&path)
        .with_context(|| format!("read current guest-launcher executable {}", path.display()))?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

fn guest_launcher_executable_path() -> Result<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        Ok(PathBuf::from("/proc/self/exe"))
    }
    #[cfg(not(target_os = "linux"))]
    {
        std::env::current_exe().context("resolve current guest-launcher executable")
    }
}

fn finalize_sp1_proposal_gas_estimator_report(
    report: &mut BenchReport,
    public_values: String,
    execution_report: &ExecutionReport,
    proposal_elf_sha256: String,
    guest_launcher_sha256: String,
) -> Result<()> {
    apply_sp1_execution_engine_metadata(report, Sp1ExecutionEngine::GasEstimator);
    report.public_values = public_values;
    report.sp1_proposal_elf_sha256 = Some(proposal_elf_sha256);
    report.guest_launcher_sha256 = Some(guest_launcher_sha256);
    apply_execution_metadata(report, execution_report);
    match report.exit_code {
        Some(0) => Ok(()),
        Some(code) => bail!("SP1 proposal gas-estimator guest exited with code {code}"),
        None => bail!("SP1 proposal gas-estimator guest exit code is missing"),
    }
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
    println!("risc0_image_id: {}", execution.image_id);
    println!("risc0_input_bytes: {}", execution.input_bytes);
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

fn write_proof_json(path: &PathBuf, proof: &Proof) -> Result<()> {
    let contents = serde_json::to_string_pretty(proof).context("serialize proof json")?;
    fs::write(path, contents).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        Args, BenchReport, ProofType, Risc0ProposalExecution, Sp1ExecutionEngine, Stage,
        apply_controlled_opcode_trace, apply_controlled_precompile_trace,
        apply_risc0_execution_metadata, apply_sp1_metadata, canonical_sp1_core_opts,
        canonicalize_sp1_core_opts, execute_opcode_lab_gas_estimator_with_opts,
        finalize_opcode_lab_execution_report, finalize_sp1_proposal_gas_estimator_report,
        guest_launcher_executable_path, install_opcode_lab_input_identity,
        new_controlled_overhead_report, parse_sp1_program, read_input, read_opcode_lab_input,
        read_opcode_lab_input_list, risc0_padded_cycles, run_controlled_state_holdout_trace,
        validate_proposal_gas_estimator_guest_elf_override,
    };
    use alloy_primitives::{Address, B256, hex};
    use clap::Parser as _;
    use raiko2_primitives::{
        OpcodeLabInput, PrecompileLabInput, PrecompileLabLane, ProofType as RaikoProofType,
        SupportedChainSpecs,
    };
    use raiko2_primitives_shasta::{GuestInput, build_proof_carry_data_from_witness_spec};
    use raiko2_prover::sp1::Sp1ExecutionMetadata;
    use sp1_sdk::ExecutionReport;
    use std::{ffi::OsStr, fs};

    #[test]
    fn parses_controlled_overhead_production_proposal_stage() {
        let args = Args::try_parse_from([
            "guest-launcher",
            "--stage",
            "controlled-overhead",
            "--proof-type",
            "sp1",
            "--mode",
            "execute",
            "--sp1-prover",
            "local",
            "--input",
            "controlled-overhead.json",
            "--jsonl-out",
            "controlled-overhead-runs.jsonl",
        ])
        .expect("parse args");

        assert_eq!(args.stage, Stage::ControlledOverhead);
        assert_eq!(args.sp1_execution_engine, Sp1ExecutionEngine::Standard);
        assert!(args.elf.is_none(), "production proposal ELF is built in");
        args.validate_sp1_execution_engine()
            .expect("controlled overhead preserves the legacy standard engine default");
        let report = serde_json::to_value(new_controlled_overhead_report(
            args.sp1_execution_engine,
            "block_base_target:8".into(),
        ))
        .expect("serialize report");
        assert_eq!(report["sp1_execution_engine"], "standard");
        assert!(report["sp1_gas_trace_chunk_threshold"].is_null());
        assert!(report["sp1_gas_trace_chunk_slots"].is_null());
    }

    #[test]
    fn controlled_overhead_accepts_local_sp1_execute_gas_estimator() {
        let args = Args::try_parse_from([
            "guest-launcher",
            "--stage",
            "controlled-overhead",
            "--proof-type",
            "sp1",
            "--mode",
            "execute",
            "--sp1-prover",
            "local",
            "--sp1-execution-engine",
            "gas-estimator",
            "--input",
            "controlled-overhead.json",
            "--jsonl-out",
            "controlled-overhead-runs.jsonl",
        ])
        .expect("parse args");

        args.validate_sp1_execution_engine()
            .expect("controlled overhead accepts the local SP1 gas estimator");
        assert!(args.elf.is_none(), "production proposal ELF is built in");

        let report =
            new_controlled_overhead_report(args.sp1_execution_engine, "block_base_target:8".into());
        let report = serde_json::to_value(report).expect("serialize report");
        assert_eq!(report["sp1_execution_engine"], "gas-estimator");
        assert_eq!(report["sp1_gas_trace_chunk_threshold"], 134_217_728);
        assert_eq!(report["sp1_gas_trace_chunk_slots"], 2);
    }

    #[test]
    fn controlled_overhead_gas_estimator_rejects_wrong_execution_guards() {
        for (proof_type, mode, prover, aggregate) in [
            ("native", "execute", "local", false),
            ("sp1", "prove", "local", false),
            ("sp1", "execute", "network", false),
            ("sp1", "execute", "local", true),
        ] {
            let mut argv = vec![
                "guest-launcher",
                "--stage",
                "controlled-overhead",
                "--proof-type",
                proof_type,
                "--mode",
                mode,
                "--sp1-prover",
                prover,
                "--sp1-execution-engine",
                "gas-estimator",
                "--input",
                "controlled-overhead.json",
                "--jsonl-out",
                "controlled-overhead-runs.jsonl",
            ];
            if aggregate {
                argv.extend(["--aggregate", "proof.json"]);
            }
            let args = Args::try_parse_from(argv).expect("parse invalid args");
            assert!(
                args.validate_sp1_execution_engine().is_err(),
                "controlled overhead accepted proof_type={proof_type} mode={mode} prover={prover} aggregate={aggregate}",
            );
        }
    }

    #[test]
    fn parses_controlled_block_production_proposal_stage() {
        let args = Args::try_parse_from([
            "guest-launcher",
            "--stage",
            "controlled-block",
            "--proof-type",
            "sp1",
            "--mode",
            "execute",
            "--sp1-prover",
            "local",
            "--input",
            "controlled-block.json",
            "--jsonl-out",
            "controlled-block-runs.jsonl",
        ])
        .expect("parse args");

        assert_eq!(args.stage, Stage::ControlledBlock);
        assert!(args.elf.is_none(), "production proposal ELF is built in");
    }

    #[test]
    fn parses_controlled_state_holdout_production_proposal_stage() {
        let args = Args::try_parse_from([
            "guest-launcher",
            "--stage",
            "controlled-state-holdout",
            "--proof-type",
            "sp1",
            "--mode",
            "execute",
            "--sp1-prover",
            "local",
            "--sp1-execution-engine",
            "gas-estimator",
            "--input",
            "controlled-state-holdout.json",
            "--jsonl-out",
            "controlled-state-holdout-runs.jsonl",
        ])
        .expect("parse args");

        assert_eq!(args.stage, Stage::ControlledStateHoldout);
        assert!(args.elf.is_none(), "production proposal ELF is built in");
        args.validate_sp1_execution_engine()
            .expect("controlled state holdouts accept the local SP1 gas estimator");
    }

    #[test]
    fn controlled_state_holdout_rejects_the_standard_execution_engine() {
        let args = Args::try_parse_from([
            "guest-launcher",
            "--stage",
            "controlled-state-holdout",
            "--proof-type",
            "sp1",
            "--mode",
            "execute",
            "--sp1-prover",
            "local",
            "--input",
            "controlled-state-holdout.json",
            "--jsonl-out",
            "controlled-state-holdout-runs.jsonl",
        ])
        .expect("parse args");

        assert!(args.validate_sp1_execution_engine().is_err());
    }

    #[test]
    fn parses_native_controlled_state_holdout_trace_stage() {
        let args = Args::try_parse_from([
            "guest-launcher",
            "--stage",
            "controlled-state-holdout-trace",
            "--proof-type",
            "native",
            "--mode",
            "execute",
            "--input",
            "controlled-state-holdout.json",
            "--jsonl-out",
            "controlled-state-holdout-trace.jsonl",
        ])
        .expect("parse args");

        assert_eq!(args.stage, Stage::ControlledStateHoldoutTrace);
        args.validate_controlled_state_holdout_trace()
            .expect("native controlled state holdout trace args");
    }

    #[test]
    fn controlled_state_holdout_trace_rejects_incompatible_flags() {
        let parse = |extra: &[&str]| {
            let mut argv = vec![
                "guest-launcher",
                "--stage",
                "controlled-state-holdout-trace",
                "--input",
                "controlled-state-holdout.json",
                "--jsonl-out",
                "controlled-state-holdout-trace.jsonl",
            ];
            argv.extend_from_slice(extra);
            Args::try_parse_from(argv).expect("parse incompatible trace args")
        };
        for extra in [
            vec!["--proof-type", "sp1"],
            vec!["--proof-type", "risc0"],
            vec!["--mode", "prove"],
            vec!["--sp1-execution-engine", "gas-estimator"],
            vec!["--aggregate", "proof.json"],
            vec!["--elf", "guest.elf"],
            vec!["--output", "proof.json"],
            vec!["--json-out", "report.json"],
            vec!["--input-list", "inputs.json"],
            vec!["--proof-mode", "compressed"],
            vec!["--sp1-prover", "local"],
            vec!["--sp1-network-mode", "mainnet"],
            vec!["--sp1-fulfillment-strategy", "hosted"],
            vec!["--sp1-cycle-limit", "1"],
            vec!["--sp1-timeout-secs", "1"],
            vec!["--risc0-execution-po2", "21"],
        ] {
            assert!(
                parse(&extra)
                    .validate_controlled_state_holdout_trace()
                    .is_err(),
                "trace stage accepted incompatible flags {extra:?}",
            );
        }

        let missing_input = Args::try_parse_from([
            "guest-launcher",
            "--stage",
            "controlled-state-holdout-trace",
            "--jsonl-out",
            "trace.jsonl",
        ])
        .expect("parse missing input args");
        assert!(
            missing_input
                .validate_controlled_state_holdout_trace()
                .is_err()
        );
        let missing_output = Args::try_parse_from([
            "guest-launcher",
            "--stage",
            "controlled-state-holdout-trace",
            "--input",
            "pair.json",
        ])
        .expect("parse missing output args");
        assert!(
            missing_output
                .validate_controlled_state_holdout_trace()
                .is_err()
        );
    }

    #[test]
    fn controlled_state_holdout_trace_writes_exactly_two_typed_rows_without_prover_gas() {
        fn assert_no_prover_gas(value: &serde_json::Value) {
            match value {
                serde_json::Value::Object(object) => {
                    assert!(!object.contains_key("proverGas"));
                    assert!(!object.contains_key("prover_gas"));
                    for child in object.values() {
                        assert_no_prover_gas(child);
                    }
                }
                serde_json::Value::Array(values) => {
                    for child in values {
                        assert_no_prover_gas(child);
                    }
                }
                _ => {}
            }
        }

        let directory = tempfile::tempdir().expect("temporary directory");
        let input_path = directory.path().join("pair.json");
        let output_path = directory.path().join("trace.jsonl");
        let spec = serde_json::json!({
            "pair_id": "witness_topology_1",
            "kind": "witness_topology",
            "scale": 1,
            "control": {"extra_account_count": 0},
            "target": {"extra_account_count": 1},
        });
        fs::write(
            &input_path,
            serde_json::to_vec(&spec).expect("serialize pair"),
        )
        .expect("write pair");
        let args = Args::try_parse_from([
            "guest-launcher",
            "--stage",
            "controlled-state-holdout-trace",
            "--proof-type",
            "native",
            "--mode",
            "execute",
            "--input",
            input_path.to_str().expect("UTF-8 input path"),
            "--jsonl-out",
            output_path.to_str().expect("UTF-8 output path"),
        ])
        .expect("parse trace args");

        run_controlled_state_holdout_trace(args).expect("run host-only trace");

        let output = fs::read_to_string(output_path).expect("read trace rows");
        assert!(output.ends_with('\n'));
        let rows = output
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("parse trace row"))
            .collect::<Vec<_>>();
        assert_eq!(rows.len(), 2);
        for (row, lane) in rows.iter().zip(["control", "target"]) {
            assert_eq!(
                row.as_object()
                    .expect("trace row object")
                    .keys()
                    .map(String::as_str)
                    .collect::<std::collections::BTreeSet<_>>(),
                std::collections::BTreeSet::from([
                    "lane",
                    "observation",
                    "pair_id",
                    "schema_version",
                    "spec",
                ]),
            );
            assert_eq!(row["schema_version"], 1);
            assert_eq!(row["pair_id"], "witness_topology_1");
            assert_eq!(row["lane"], lane);
            assert_eq!(row["spec"], spec);
            assert_eq!(row["observation"]["lane"], lane);
            assert_eq!(row["observation"]["pair_id"], "witness_topology_1");
            assert_no_prover_gas(row);
        }
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
        assert_eq!(args.sp1_execution_engine, Sp1ExecutionEngine::Standard);
        assert_eq!(
            args.elf.expect("elf path").display().to_string(),
            "crates/guests/elf/sp1_opcode_lab.elf"
        );
    }

    #[test]
    fn gas_estimator_accepts_local_sp1_execute_for_every_supported_stage() {
        for stage in [
            "proposal",
            "opcode-lab",
            "revm-opcode-lab",
            "controlled-overhead",
            "controlled-block",
            "controlled-state-holdout",
        ] {
            let args = Args::try_parse_from([
                "guest-launcher",
                "--stage",
                stage,
                "--proof-type",
                "sp1",
                "--mode",
                "execute",
                "--sp1-prover",
                "local",
                "--sp1-execution-engine",
                "gas-estimator",
                "--input",
                "/tmp/input.json",
            ])
            .expect("parse gas-estimator args");

            args.validate_sp1_execution_engine()
                .expect("valid gas-estimator selection");
        }
    }

    #[test]
    fn gas_estimator_rejects_non_lab_prove_non_local_and_aggregate_usage() {
        let invalid = [
            ("precompile-lab", "sp1", "execute", "local", false),
            ("proposal", "sp1", "prove", "local", false),
            ("proposal", "sp1", "execute", "network", false),
            ("proposal", "native", "execute", "local", false),
            ("proposal", "sp1", "execute", "local", true),
            ("opcode-lab", "sp1", "prove", "local", false),
            ("opcode-lab", "sp1", "execute", "network", false),
            ("opcode-lab", "native", "execute", "local", false),
            ("opcode-lab", "sp1", "execute", "local", true),
        ];
        for (stage, proof_type, mode, prover, aggregate) in invalid {
            let mut argv = vec![
                "guest-launcher",
                "--stage",
                stage,
                "--proof-type",
                proof_type,
                "--mode",
                mode,
                "--sp1-prover",
                prover,
                "--sp1-execution-engine",
                "gas-estimator",
                "--input",
                "/tmp/input.json",
            ];
            if aggregate {
                argv.extend(["--aggregate", "/tmp/proof.json"]);
            }
            let args = Args::try_parse_from(argv).expect("parse invalid selection");
            assert!(
                args.validate_sp1_execution_engine().is_err(),
                "gas estimator unexpectedly accepted stage={} mode={:?} proof_type={:?}",
                args.stage.as_str(),
                args.mode,
                args.proof_type,
            );
        }
    }

    #[test]
    fn proposal_gas_estimator_rejects_alternate_elf() {
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
            "--sp1-execution-engine",
            "gas-estimator",
            "--elf",
            "/tmp/proposal.elf",
            "--input",
            "/tmp/input.json",
        ])
        .expect("parse proposal gas-estimator args");

        let error = args
            .validate_standard_guest_artifacts()
            .expect_err("proposal gas estimation must use the production ELF");
        assert!(error.to_string().contains("production guest pair"));
    }

    #[test]
    fn proposal_gas_estimator_rejects_guest_elf_directory_override() {
        let error = validate_proposal_gas_estimator_guest_elf_override(Some(OsStr::new(
            "/tmp/stale-guest-artifacts",
        )))
        .expect_err("proposal gas estimation must reject an environment override");

        assert!(error.to_string().contains("RAIKO2_GUEST_ELF_DIR"));
        validate_proposal_gas_estimator_guest_elf_override(None)
            .expect("the production guest directory is accepted");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn proposal_gas_estimator_hashes_the_executing_launcher_inode() {
        assert_eq!(
            guest_launcher_executable_path().expect("resolve executing launcher inode"),
            std::path::PathBuf::from("/proc/self/exe")
        );
    }

    #[test]
    fn benchmark_report_records_sp1_execution_engine() {
        let mut report = BenchReport::new("opcode-lab", "execute", "core", "input.json".into());
        let standard = serde_json::to_value(&report).expect("serialize standard report");
        assert_eq!(standard["sp1_execution_engine"], "standard");
        assert!(standard["sp1_gas_trace_chunk_threshold"].is_null());
        assert!(standard["sp1_gas_trace_chunk_slots"].is_null());
        assert!(standard.get("sp1_proposal_elf_sha256").is_none());
        assert!(standard.get("guest_launcher_sha256").is_none());

        report.sp1_execution_engine = Sp1ExecutionEngine::GasEstimator.as_str();
        report.sp1_gas_trace_chunk_threshold = Some(134_217_728);
        report.sp1_gas_trace_chunk_slots = Some(2);
        let estimator = serde_json::to_value(&report).expect("serialize estimator report");
        assert_eq!(estimator["sp1_execution_engine"], "gas-estimator");
        assert_eq!(estimator["sp1_gas_trace_chunk_threshold"], 134_217_728);
        assert_eq!(estimator["sp1_gas_trace_chunk_slots"], 2);
    }

    #[test]
    fn opcode_lab_report_binds_canonical_bincode_input_identity() {
        use sha2::{Digest as _, Sha256};

        let input = OpcodeLabInput {
            case: "synthetic_anchor_probe_pop".into(),
            scenario: "anchor_target_".into(),
            opcode: 0x50,
            target_count: 1024,
            target_raw_gas: 2,
            tx_gas_limit: Some(100_000),
            bytecode: vec![0x00],
            generator_max_count: Some(131_072),
            fixed_bytecode_len: Some(1),
        };
        let encoded = bincode::serialize(&input).unwrap();
        let expected_hash = format!("0x{}", hex::encode(Sha256::digest(&encoded)));
        let mut report = BenchReport::new("opcode-lab", "execute", "core", "input.json".into());

        install_opcode_lab_input_identity(&mut report, &input).unwrap();

        assert_eq!(
            report.guest_input_sha256.as_deref(),
            Some(expected_hash.as_str())
        );
        assert_eq!(report.guest_input_bincode_length, Some(encoded.len()));
    }

    #[test]
    fn opcode_lab_report_rejects_nonzero_guest_exit() {
        let mut execution = ExecutionReport::default();
        execution.exit_code = 3;
        let mut report = BenchReport::new("opcode-lab", "execute", "core", "input.json".into());

        let error = finalize_opcode_lab_execution_report(&mut report, &execution)
            .expect_err("nonzero guest exit must fail closed");

        assert!(error.to_string().contains("guest exited with code 3"));
        assert_eq!(report.exit_code, Some(3));
    }

    #[test]
    fn canonical_gas_estimator_options_ignore_ambient_sp1_overrides() {
        // Model every environment-overridable field with deliberately noncanonical values, then
        // prove the formal sampling normalization replaces all of them without mutating process env.
        let ambient = sp1_core_executor::SP1CoreOpts {
            minimal_trace_chunk_threshold: 200_000,
            gas_trace_chunk_threshold: 100_000,
            trace_chunk_slots: 8,
            gas_trace_chunk_slots: 9,
            memory_limit: 123_456,
            shard_size: 1_024,
            sharding_threshold: sp1_core_executor::ShardingThreshold {
                element_threshold: 2_048,
                height_threshold: 4_096,
            },
            ..sp1_core_executor::SP1CoreOpts::default()
        };
        let opts = canonicalize_sp1_core_opts(ambient);

        assert_eq!(opts.minimal_trace_chunk_threshold, 16_777_216);
        assert_eq!(opts.gas_trace_chunk_threshold, 134_217_728);
        assert_eq!(opts.trace_chunk_slots, 5);
        assert_eq!(opts.gas_trace_chunk_slots, 2);
        assert_eq!(opts.memory_limit, 24 * 1024 * 1024 * 1024);
        assert_eq!(opts.shard_size, 1 << 24);
        assert_eq!(
            opts.sharding_threshold.element_threshold,
            (1 << 28) + (1 << 27)
        );
        assert_eq!(opts.sharding_threshold.height_threshold, 1 << 22);
    }

    fn current_revm_add_32_fixture() -> (std::sync::Arc<sp1_core_executor::Program>, OpcodeLabInput)
    {
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let elf = std::fs::read(repo.join("crates/guests/elf/sp1_revm_opcode_lab.elf"))
            .expect("read checked-in revm opcode lab ELF");
        let input_path = repo.join("bin/guest-launcher/tests/fixtures/revm-opcode-lab-add-32.json");
        let input = read_opcode_lab_input(&input_path).expect("read checked-in add/count-32 input");
        (
            parse_sp1_program(&elf).expect("parse checked-in ELF"),
            input,
        )
    }

    fn assert_add_32_execution_surface(
        public_values: &sp1_sdk::SP1PublicValues,
        report: &sp1_sdk::ExecutionReport,
        expected_gas: u64,
    ) {
        assert_eq!(
            public_values.raw(),
            "0x9318bc580c9b2aa315a8649bd205867ef84a5d28fecdb187ec57ba86f409ec16"
        );
        assert_eq!(report.gas(), Some(expected_gas));
        assert_eq!(report.total_instruction_count(), 1_597_491);
        assert_eq!(report.total_syscall_count(), 35);
        assert_eq!(report.exit_code, 0);
    }

    #[test]
    fn canonical_gas_estimator_matches_checked_in_add_32_baseline() {
        let (program, input) = current_revm_add_32_fixture();
        let (public_values, report) =
            execute_opcode_lab_gas_estimator_with_opts(program, &input, canonical_sp1_core_opts())
                .expect("execute canonical gas estimator");

        // Hand-checked against the standard SP1 6.3 execution baseline for this tracked fixture.
        assert_add_32_execution_surface(&public_values, &report, 1_443_869);
    }

    #[test]
    fn gas_estimator_sums_all_forced_small_chunks() {
        let (program, input) = current_revm_add_32_fixture();
        let mut opts = canonical_sp1_core_opts();
        opts.gas_trace_chunk_threshold = 100_000;
        let (public_values, report) =
            execute_opcode_lab_gas_estimator_with_opts(program, &input, opts)
                .expect("execute forced multi-chunk gas estimator");

        assert_add_32_execution_surface(&public_values, &report, 1_595_314);
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
    fn standard_sp1_proposal_rejects_single_elf_override() {
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
            "/tmp/proposal.elf",
            "--input",
            "/tmp/guest-input.json",
        ])
        .expect("parse args");

        let error = args
            .validate_standard_guest_artifacts()
            .expect_err("single standard ELF override must be rejected");
        assert!(error.to_string().contains("RAIKO2_GUEST_ELF_DIR"));
    }

    #[test]
    fn standard_sp1_proposal_requires_compressed_proof() {
        let args = Args::try_parse_from([
            "guest-launcher",
            "--stage",
            "proposal",
            "--proof-type",
            "sp1",
            "--mode",
            "prove",
            "--proof-mode",
            "plonk",
            "--input",
            "/tmp/guest-input.json",
        ])
        .expect("parse args");

        let error = args
            .validate_standard_guest_artifacts()
            .expect_err("proposal proof must be compressed");
        assert!(error.to_string().contains("--proof-mode compressed"));
    }

    #[test]
    fn standard_sp1_aggregation_requires_plonk_proof() {
        let args = Args::try_parse_from([
            "guest-launcher",
            "--stage",
            "proposal",
            "--proof-type",
            "sp1",
            "--mode",
            "prove",
            "--proof-mode",
            "compressed",
            "--aggregate",
            "/tmp/proof.json",
            "--output",
            "/tmp/aggregation.json",
        ])
        .expect("parse args");

        let error = args
            .validate_standard_guest_artifacts()
            .expect_err("aggregation proof must be plonk");
        assert!(error.to_string().contains("--proof-mode plonk"));
    }

    #[test]
    fn production_sp1_metadata_preserves_benchmark_fields() {
        let metadata = Sp1ExecutionMetadata {
            zkvm: "sp1".to_string(),
            mode: "execute".to_string(),
            public_values: "0x12".to_string(),
            exit_code: 0,
            gas: Some(7),
            total_instruction_count: 11,
            total_syscall_count: 3,
            touched_memory_addresses: 5,
            cycle_tracker: Vec::new(),
            invocation_tracker: Vec::new(),
            opcode_counts: Vec::new(),
            syscall_counts: Vec::new(),
        };
        let mut report = BenchReport::new(
            "proposal",
            "execute",
            "compressed",
            "input.json".to_string(),
        );

        apply_sp1_metadata(&mut report, &metadata);

        assert_eq!(report.public_values, "0x12");
        assert_eq!(report.gas, Some(7));
        assert_eq!(report.total_instruction_count, Some(11));
    }

    #[test]
    fn proposal_gas_estimator_report_preserves_provenance_and_join_fields() {
        let mut execution = ExecutionReport::default();
        execution.exit_code = 0;
        let mut report = BenchReport::new(
            "proposal",
            "execute",
            "compressed",
            "input.json".to_string(),
        );
        report.guest_input_sha256 = Some(format!("0x{}", "11".repeat(32)));
        report.guest_input_bincode_length = Some(1234);

        finalize_sp1_proposal_gas_estimator_report(
            &mut report,
            "0x1234".into(),
            &execution,
            "22".repeat(32),
            "33".repeat(32),
        )
        .expect("finalize proposal estimator report");

        let serialized = serde_json::to_value(report).expect("serialize proposal report");
        assert_eq!(serialized["stage"], "proposal");
        assert_eq!(serialized["mode"], "execute");
        assert_eq!(serialized["sp1_execution_engine"], "gas-estimator");
        assert_eq!(serialized["sp1_gas_trace_chunk_threshold"], 134_217_728);
        assert_eq!(serialized["sp1_gas_trace_chunk_slots"], 2);
        assert_eq!(
            serialized["guest_input_sha256"],
            format!("0x{}", "11".repeat(32))
        );
        assert_eq!(serialized["guest_input_bincode_length"], 1234);
        assert_eq!(serialized["public_values"], "0x1234");
        assert_eq!(serialized["exit_code"], 0);
        assert_eq!(serialized["sp1_proposal_elf_sha256"], "22".repeat(32));
        assert_eq!(serialized["guest_launcher_sha256"], "33".repeat(32));
    }

    #[test]
    fn revm_opcode_report_contains_the_host_executed_trace_identity() {
        let input = OpcodeLabInput {
            case: "add".into(),
            scenario: "arithmetic".into(),
            opcode: 0x01,
            target_count: 1,
            target_raw_gas: 3,
            tx_gas_limit: Some(1_000_024),
            bytecode: vec![0x60, 0x01, 0x60, 0x02, 0x01, 0x00],
            generator_max_count: Some(8),
            fixed_bytecode_len: Some(6),
        };
        let mut report =
            BenchReport::new("revm-opcode-lab", "execute", "core", "input.json".into());

        apply_controlled_opcode_trace(&mut report, Stage::RevmOpcodeLab, &input).unwrap();

        let serialized = serde_json::to_value(report).unwrap();
        let trace = serialized["controlled_trace"].as_object().unwrap();
        assert_eq!(trace["executed_target_count"], 1);
        assert_eq!(trace["executed_target_raw_gas"], 3);
        assert_eq!(trace["backend_input_sha256"].as_str().unwrap().len(), 64);
        assert_eq!(trace["workload_id"].as_str().unwrap().len(), 64);
        assert_eq!(
            serialized["guest_input_sha256"],
            format!("0x{}", trace["backend_input_sha256"].as_str().unwrap())
        );
        assert_eq!(
            serialized["guest_input_bincode_length"],
            trace["backend_input_len"]
        );
    }

    #[test]
    fn revm_opcode_trace_rejects_a_conflicting_canonical_input_identity() {
        let input = OpcodeLabInput {
            case: "add".into(),
            scenario: "arithmetic".into(),
            opcode: 0x01,
            target_count: 1,
            target_raw_gas: 3,
            tx_gas_limit: Some(1_000_024),
            bytecode: vec![0x60, 0x01, 0x60, 0x02, 0x01, 0x00],
            generator_max_count: Some(8),
            fixed_bytecode_len: Some(6),
        };
        let mut report =
            BenchReport::new("revm-opcode-lab", "execute", "core", "input.json".into());
        install_opcode_lab_input_identity(&mut report, &input).unwrap();
        report.guest_input_sha256 = Some(format!("0x{}", "00".repeat(32)));

        let error = apply_controlled_opcode_trace(&mut report, Stage::RevmOpcodeLab, &input)
            .expect_err("conflicting canonical and trace identities must fail closed");

        assert!(error.to_string().contains("input identity differs"));
    }

    #[test]
    fn precompile_report_contains_typed_pair_and_backend_input_identities() {
        let input = PrecompileLabInput {
            case: "identity".into(),
            scenario: "free text".into(),
            lane: PrecompileLabLane::Control,
            address: 4,
            target_count: 2,
            input_size: 4,
            target_raw_gas: 18,
            expected_output_size: Some(4),
            input: vec![1, 2, 3, 4],
        };
        let mut report = BenchReport::new("precompile-lab", "execute", "core", "input.json".into());

        apply_controlled_precompile_trace(&mut report, &input).unwrap();

        let serialized = serde_json::to_value(report).unwrap();
        let trace = serialized["controlled_trace"].as_object().unwrap();
        assert_eq!(trace["kind"], "precompile");
        assert_eq!(trace["lane"], "control");
        assert_eq!(trace["address"], 4);
        assert_eq!(trace["output_len"], 4);
        assert_eq!(trace["workload_id"].as_str().unwrap().len(), 64);
        assert_eq!(trace["pair_id"].as_str().unwrap().len(), 64);
        assert_eq!(trace["backend_input_sha256"].as_str().unwrap().len(), 64);
        assert_eq!(
            serialized["guest_input_sha256"],
            format!("0x{}", trace["backend_input_sha256"].as_str().unwrap())
        );
        assert_eq!(
            serialized["guest_input_bincode_length"],
            trace["backend_input_len"]
        );
    }

    #[test]
    fn risc0_padded_cycles_sum_segment_po2s() {
        assert_eq!(risc0_padded_cycles([10, 11, 10]), 4096);
    }

    #[test]
    fn risc0_metadata_reports_encoded_input_bytes() {
        let mut report = BenchReport::new(
            "proposal",
            "execute",
            "compressed",
            "input.json".to_string(),
        );
        let execution = Risc0ProposalExecution {
            public_values: "0x12".to_string(),
            wall_time_ms: 7,
            image_id: "0xabcd".to_string(),
            input_bytes: 123,
            user_cycles: 456,
            padded_cycles: 512,
            segment_count: 1,
            po2_counts: Vec::new(),
        };

        apply_risc0_execution_metadata(&mut report, &execution);

        assert_eq!(report.risc0_image_id.as_deref(), Some("0xabcd"));
        assert_eq!(report.risc0_input_bytes, Some(123));
        assert!(
            report
                .workload_metrics
                .iter()
                .any(|entry| { entry.label == "risc0_input_bytes" && entry.count == 123 })
        );
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
            build_proof_carry_data_from_witness_spec(&input, RaikoProofType::Native)
                .expect("build carry data");
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
