use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use alloy_primitives::{Address, B256, hex};
use anyhow::{Context, Result, bail};
use clap::{Parser, ValueEnum};
use raiko2_pipeline::forks::shasta::SP1_SHASTA_BACKEND;
use raiko2_pipeline::{NativeBackend, ProofStage, ProverBackend};
use raiko2_primitives::{Proof, ProofType as RaikoProofType};
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
use serde::Serialize;
use sp1_sdk::utils::setup_logger;
use sp1_sdk::{
    ExecutionReport, NetworkProver, Prover as _, ProverClient, SP1Proof, SP1ProofMode,
    SP1ProofWithPublicValues, SP1ProvingKey, SP1Stdin, SP1VerifyingKey,
};

#[derive(Parser)]
#[command(name = "guest-launcher")]
#[command(about = "Run SP1 guest programs locally with JSON inputs", long_about = None)]
struct Args {
    /// Path to the input JSON file.
    #[arg(long)]
    input: Option<PathBuf>,
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
}

#[derive(Clone, Copy, Debug, ValueEnum, PartialEq, Eq)]
enum Mode {
    Execute,
    Prove,
}

#[derive(Clone, Copy, Debug, ValueEnum, PartialEq, Eq)]
enum ProofType {
    Native,
    Sp1,
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

#[derive(Debug, Serialize)]
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
    exit_code: Option<u64>,
    gas: Option<u64>,
    total_instruction_count: Option<u64>,
    total_syscall_count: Option<u64>,
    touched_memory_addresses: Option<u64>,
    cycle_tracker: Vec<BenchCycleEntry>,
    invocation_tracker: Vec<BenchCountEntry>,
    opcode_counts: Vec<BenchCountEntry>,
    syscall_counts: Vec<BenchCountEntry>,
    memory_snapshots: Vec<BenchMemoryEntry>,
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
            ProofType::Sp1 => RaikoProofType::Sp1,
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
            timeout_secs: self.sp1_timeout_secs,
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

fn apply_execution_metadata(report: &mut BenchReport, execution_report: &ExecutionReport) {
    report.exit_code = Some(0);
    report.gas = execution_report.gas;
    report.total_instruction_count = Some(execution_report.total_instruction_count());
    report.total_syscall_count = Some(execution_report.total_syscall_count());
    report.touched_memory_addresses = Some(execution_report.touched_memory_addresses);
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

#[tokio::main]
async fn main() -> Result<()> {
    // Enable SP1 runtime logs (includes guest `println!` output).
    // Use `RUST_LOG=info` (or `debug`) when running this binary to see them.
    setup_logger();

    let args = Args::parse();

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

async fn run_proposal(args: Args) -> Result<()> {
    let input_path = args.input.clone().context("missing --input")?;
    let proof_mode = args.effective_proof_mode();
    let mut report = BenchReport {
        stage: "proposal",
        mode: args.mode.as_str(),
        proof_mode: proof_mode.as_str(),
        input: input_path.display().to_string(),
        public_values: String::new(),
        wall_time_ms: 0,
        exit_code: None,
        gas: None,
        total_instruction_count: None,
        total_syscall_count: None,
        touched_memory_addresses: None,
        cycle_tracker: Vec::new(),
        invocation_tracker: Vec::new(),
        opcode_counts: Vec::new(),
        syscall_counts: Vec::new(),
        memory_snapshots: Vec::new(),
    };
    record_memory_snapshot(&mut report, "proposal:start");
    let input = read_input(&input_path, args.proof_type)?;
    record_memory_snapshot(&mut report, "proposal:after_read_input");

    match args.proof_type {
        ProofType::Sp1 => run_sp1_proposal(args, input_path, input, report).await,
        ProofType::Native => run_native_proposal(args, input_path, input, report).await,
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

    let proposal_elf = SP1_SHASTA_BACKEND
        .elf(ProofStage::Proposal)
        .context("load SP1 proposal ELF")?;
    let elf = SP1_SHASTA_BACKEND
        .elf(ProofStage::Aggregation)
        .context("load SP1 aggregation ELF")?;
    let start = Instant::now();
    let (proof, proposal_vk) = match sp1_config.prover {
        Sp1ProverMode::Mock => {
            let prover = ProverClient::builder().mock().build();
            let (_, proposal_vk) = prover.setup(proposal_elf);
            for proof in sp1_proofs {
                match proof {
                    SP1Proof::Compressed(reduce_proof) => {
                        stdin.write_proof(*reduce_proof, proposal_vk.vk.clone());
                    }
                    _ => anyhow::bail!("aggregation requires compressed proofs"),
                }
            }
            let (pk, vk) = prover.setup(elf);
            (
                Sp1ProofOutput {
                    proof: prover
                        .prove(&pk, &stdin)
                        .mode(proof_mode.into())
                        .run()
                        .context("prove failed")?,
                    vkey: vk,
                },
                proposal_vk,
            )
        }
        Sp1ProverMode::Local => {
            let prover = ProverClient::builder().cpu().build();
            let (_, proposal_vk) = prover.setup(proposal_elf);
            for proof in sp1_proofs {
                match proof {
                    SP1Proof::Compressed(reduce_proof) => {
                        stdin.write_proof(*reduce_proof, proposal_vk.vk.clone());
                    }
                    _ => anyhow::bail!("aggregation requires compressed proofs"),
                }
            }
            let (pk, vk) = prover.setup(elf);
            (
                Sp1ProofOutput {
                    proof: prover
                        .prove(&pk, &stdin)
                        .mode(proof_mode.into())
                        .run()
                        .context("prove failed")?,
                    vkey: vk,
                },
                proposal_vk,
            )
        }
        Sp1ProverMode::Network => {
            let prover = build_sp1_network_prover(&sp1_config)?;
            let (_, proposal_vk) = prover.setup(proposal_elf);
            for proof in sp1_proofs {
                match proof {
                    SP1Proof::Compressed(reduce_proof) => {
                        stdin.write_proof(*reduce_proof, proposal_vk.vk.clone());
                    }
                    _ => anyhow::bail!("aggregation requires compressed proofs"),
                }
            }
            let (pk, _) = prover.setup(elf);
            (
                request_network_proof(&prover, &pk, stdin, proof_mode.into(), &sp1_config)
                    .await
                    .context("prove failed")?,
                proposal_vk,
            )
        }
    };
    let report = BenchReport {
        stage: "aggregation",
        mode: args.mode.as_str(),
        proof_mode: proof_mode.as_str(),
        input: output_path.display().to_string(),
        public_values: proof.proof.public_values.raw(),
        wall_time_ms: u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX),
        exit_code: None,
        gas: None,
        total_instruction_count: None,
        total_syscall_count: None,
        touched_memory_addresses: None,
        cycle_tracker: Vec::new(),
        invocation_tracker: Vec::new(),
        opcode_counts: Vec::new(),
        syscall_counts: Vec::new(),
        memory_snapshots: Vec::new(),
    };

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

fn build_sp1_network_prover(config: &Sp1Config) -> Result<NetworkProver> {
    let private_key = std::env::var("NETWORK_PRIVATE_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .context("NETWORK_PRIVATE_KEY must be set for sp1 network proving")?;
    let builder = ProverClient::builder().network().private_key(&private_key);
    let builder = if let Some(rpc_url) = config.rpc_url.as_deref() {
        builder.rpc_url(rpc_url)
    } else {
        builder
    };
    Ok(builder.build())
}

async fn request_network_proof(
    prover: &NetworkProver,
    pk: &SP1ProvingKey,
    stdin: SP1Stdin,
    proof_mode: SP1ProofMode,
    config: &Sp1Config,
) -> Result<Sp1ProofOutput> {
    let timeout = Duration::from_secs(config.timeout_secs);
    let request_id = prover
        .prove(pk, &stdin)
        .mode(proof_mode)
        .strategy(config.fulfillment_strategy.into())
        .skip_simulation(config.skip_simulation)
        .cycle_limit(config.cycle_limit)
        .timeout(timeout)
        .request_async()
        .await
        .context("request SP1 network proof")?;
    eprintln!("sp1 request_id: {request_id}");
    let proof = prover
        .wait_proof(request_id, Some(timeout), None)
        .await
        .context("wait for SP1 network proof")?;
    Ok(Sp1ProofOutput {
        proof,
        vkey: pk.vk.clone(),
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

    let elf = SP1_SHASTA_BACKEND
        .elf(ProofStage::Proposal)
        .context("load SP1 proposal ELF")?;
    report.input = input_path.display().to_string();
    record_memory_snapshot(&mut report, "proposal:after_load_elf");

    match args.mode {
        Mode::Execute => {
            let prover = match sp1_config.prover {
                Sp1ProverMode::Mock => ProverClient::builder().mock().build(),
                Sp1ProverMode::Local => ProverClient::builder().cpu().build(),
                Sp1ProverMode::Network => {
                    anyhow::bail!("sp1.mode=execute does not support sp1.prover=network")
                }
            };
            let start = Instant::now();
            record_memory_snapshot(&mut report, "proposal:before_execute_run");
            let (public_values, execution_report) = prover
                .execute(elf, &stdin)
                .run()
                .context("execute failed")?;
            record_memory_snapshot(&mut report, "proposal:after_execute_run");
            report.wall_time_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
            report.public_values = public_values.raw();
            apply_execution_metadata(&mut report, &execution_report);
            record_memory_snapshot(&mut report, "proposal:after_apply_execution_metadata");

            println!("public_values: {}", report.public_values);
            if !execution_report.cycle_tracker.is_empty() {
                println!("cycle_tracker:");
                for (label, cycles) in execution_report.cycle_tracker {
                    println!("  {label}: {cycles}");
                    report.cycle_tracker.push(BenchCycleEntry { label, cycles });
                }
            }
        }
        Mode::Prove => {
            let start = Instant::now();
            match sp1_config.prover {
                Sp1ProverMode::Mock => {
                    let prover = ProverClient::builder().mock().build();
                    record_memory_snapshot(&mut report, "proposal:before_setup");
                    let (pk, vk) = prover.setup(elf);
                    record_memory_snapshot(&mut report, "proposal:after_setup");
                    let proof = prover
                        .prove(&pk, &stdin)
                        .mode(proof_mode.into())
                        .cycle_limit(sp1_config.cycle_limit)
                        .run()
                        .context("prove failed")?;
                    record_memory_snapshot(&mut report, "proposal:after_prove_run");
                    report.wall_time_ms =
                        u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
                    report.public_values = proof.public_values.raw();
                    println!("public_values: {}", report.public_values);

                    if let Some(path) = &args.output {
                        let output =
                            build_sp1_proposal_output(&proof, &vk, Some(&input.proof_carry_data))?;
                        write_proof_json(path, &output)?;
                    }
                }
                Sp1ProverMode::Local => {
                    let prover = ProverClient::builder().cpu().build();
                    record_memory_snapshot(&mut report, "proposal:before_setup");
                    let (pk, vk) = prover.setup(elf);
                    record_memory_snapshot(&mut report, "proposal:after_setup");
                    let proof = prover
                        .prove(&pk, &stdin)
                        .mode(proof_mode.into())
                        .cycle_limit(sp1_config.cycle_limit)
                        .run()
                        .context("prove failed")?;
                    record_memory_snapshot(&mut report, "proposal:after_prove_run");
                    report.wall_time_ms =
                        u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
                    report.public_values = proof.public_values.raw();
                    println!("public_values: {}", report.public_values);

                    if let Some(path) = &args.output {
                        let output =
                            build_sp1_proposal_output(&proof, &vk, Some(&input.proof_carry_data))?;
                        write_proof_json(path, &output)?;
                    }
                }
                Sp1ProverMode::Network => {
                    let prover = build_sp1_network_prover(&sp1_config)?;
                    record_memory_snapshot(&mut report, "proposal:before_setup");
                    let (pk, _) = prover.setup(elf);
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
    use super::{ProofType, parse_image_id_from_uuid, read_input};
    use alloy_primitives::{Address, B256};
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
