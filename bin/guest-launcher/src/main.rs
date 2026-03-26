use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use alloy_primitives::{Address, B256, hex};
use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use raiko2_pipeline::forks::shasta::SP1_SHASTA_BACKEND;
use raiko2_pipeline::{NativeBackend, ProofStage, ProverBackend};
use raiko2_primitives::Proof;
use raiko2_primitives_shasta::build_proof_carry_data;
use raiko2_primitives_shasta::decode_proof_carry_data;
use raiko2_primitives_shasta::encode_proof_carry_data;
use raiko2_primitives_shasta::instance::words_to_bytes_be;
use raiko2_primitives_shasta::{GuestInput, ShastaZkAggregationGuestInput};
use raiko2_protocol_shasta::libhash::hash_shasta_subproof_input;
use raiko2_protocol_shasta::shasta::ProofCarryData;
use raiko2_prover::Prover;
use raiko2_prover::native::NativeProver;
use serde::Serialize;
use sp1_sdk::utils::setup_logger;
use sp1_sdk::{
    Elf, ExecutionReport, HashableKey, ProveRequest as _, Prover as _, ProverClient,
    ProvingKey as _, SP1Proof, SP1ProofMode, SP1ProofWithPublicValues, SP1Stdin, SP1VerifyingKey,
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
    /// Proof mode when generating proofs.
    #[arg(long, value_enum, default_value = "plonk")]
    proof_mode: ProofMode,
    /// Proof backend to use.
    #[arg(long, value_enum, default_value = "native")]
    proof_type: ProofType,
    /// Path to write proof JSON output.
    #[arg(long)]
    output: Option<PathBuf>,

    /// Optional path to write a JSON benchmark report.
    #[arg(long)]
    json_out: Option<PathBuf>,
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

#[derive(Clone, Copy, Debug, ValueEnum)]
enum ProofMode {
    Core,
    Compressed,
    Plonk,
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

fn count_entries(entries: impl Iterator<Item = (String, u64)>) -> Vec<BenchCountEntry> {
    let mut entries = entries
        .filter_map(|(label, count)| (count > 0).then_some(BenchCountEntry { label, count }))
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.label.cmp(&right.label));
    entries
}

fn apply_execution_metadata(report: &mut BenchReport, execution_report: &ExecutionReport) {
    report.exit_code = Some(execution_report.exit_code);
    report.gas = execution_report.gas();
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

fn read_input(path: &PathBuf) -> Result<GuestInput> {
    let contents = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let mut input: GuestInput = serde_json::from_str(&contents).context("parse input JSON")?;
    if input.proof_carry_data == ProofCarryData::default() && !input.witnesses.is_empty() {
        input.proof_carry_data = build_proof_carry_data(&input);
    }
    Ok(input)
}

async fn run_proposal(args: Args) -> Result<()> {
    let input_path = args.input.clone().context("missing --input")?;
    let input = read_input(&input_path)?;

    match args.proof_type {
        ProofType::Sp1 => run_sp1_proposal(args, input_path, input).await,
        ProofType::Native => run_native_proposal(args, input_path, input).await,
    }
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

    let proofs = read_proofs(&args.aggregate)?;
    let (aggregation_input, sp1_proofs, image_id) = build_aggregation_inputs(&proofs)?;

    let mut stdin = SP1Stdin::new();
    stdin.write(&aggregation_input);

    let proposal_elf = Elf::from(
        SP1_SHASTA_BACKEND
            .elf(ProofStage::Proposal)
            .context("load SP1 proposal ELF")?,
    );
    let prover = ProverClient::from_env().await;
    let proposal_pk = prover
        .setup(proposal_elf)
        .await
        .context("setup SP1 proposal key")?;
    let proposal_vk = proposal_pk.verifying_key().clone();

    for proof in sp1_proofs {
        match proof.proof {
            SP1Proof::Compressed(reduce_proof) => {
                stdin.write_proof(*reduce_proof, proposal_vk.vk.clone());
            }
            _ => {
                anyhow::bail!("aggregation requires compressed proofs");
            }
        }
    }

    let elf = Elf::from(
        SP1_SHASTA_BACKEND
            .elf(ProofStage::Aggregation)
            .context("load SP1 aggregation ELF")?,
    );
    let pk = prover
        .setup(elf)
        .await
        .context("setup SP1 aggregation key")?;
    let start = Instant::now();
    let proof = prover
        .prove(&pk, stdin)
        .mode(args.proof_mode.into())
        .await
        .context("prove failed")?;
    let report = BenchReport {
        stage: "aggregation",
        mode: args.mode.as_str(),
        proof_mode: args.proof_mode.as_str(),
        input: output_path.display().to_string(),
        public_values: proof.public_values.raw(),
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
    };

    let output = build_sp1_proof_output(&proof, pk.verifying_key(), None)?;
    write_proof_json(output_path, &output)?;

    println!("public_values: {}", report.public_values);

    if let Some(path) = &args.json_out {
        let contents = serde_json::to_string_pretty(&report).context("serialize bench report")?;
        fs::write(path, contents).with_context(|| format!("write {}", path.display()))?;
    }

    println!("image_id: 0x{}", hex::encode(words_to_bytes_be(&image_id)));

    Ok(())
}

fn build_sp1_proof_output(
    proof: &SP1ProofWithPublicValues,
    vk: &SP1VerifyingKey,
    carry: Option<&ProofCarryData>,
) -> Result<Proof> {
    let public_values = proof.public_values.as_slice();
    let input_hash = if public_values.len() >= 32 {
        B256::from_slice(&public_values[..32])
    } else {
        B256::default()
    };
    let proof_bytes = bincode::serialize(proof).context("serialize SP1 proof")?;
    let extra_data = match carry {
        Some(data) => Some(encode_proof_carry_data(data)?),
        None => None,
    };

    Ok(Proof {
        proof: Some(hex::encode_prefixed(&proof_bytes)),
        input: Some(input_hash),
        uuid: Some(vk.bytes32().to_string()),
        extra_data,
        ..Default::default()
    })
}

async fn run_sp1_proposal(args: Args, input_path: PathBuf, input: GuestInput) -> Result<()> {
    let mut stdin = SP1Stdin::new();
    stdin.write(&input);

    let elf = Elf::from(
        SP1_SHASTA_BACKEND
            .elf(ProofStage::Proposal)
            .context("load SP1 proposal ELF")?,
    );

    let prover = ProverClient::from_env().await;

    let mut report = BenchReport {
        stage: "proposal",
        mode: args.mode.as_str(),
        proof_mode: args.proof_mode.as_str(),
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
    };

    match args.mode {
        Mode::Execute => {
            let start = Instant::now();
            let (public_values, execution_report) =
                prover.execute(elf, stdin).await.context("execute failed")?;
            report.wall_time_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
            report.public_values = public_values.raw();
            apply_execution_metadata(&mut report, &execution_report);

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
            let pk = prover.setup(elf).await.context("setup SP1 proposal key")?;
            let start = Instant::now();
            let proof = prover
                .prove(&pk, stdin)
                .mode(args.proof_mode.into())
                .await
                .context("prove failed")?;
            report.wall_time_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
            report.public_values = proof.public_values.raw();
            println!("public_values: {}", report.public_values);

            if let Some(path) = &args.output {
                let output = build_sp1_proof_output(
                    &proof,
                    pk.verifying_key(),
                    Some(&input.proof_carry_data),
                )?;
                write_proof_json(path, &output)?;
            }
        }
    }

    if let Some(path) = &args.json_out {
        let contents = serde_json::to_string_pretty(&report).context("serialize bench report")?;
        fs::write(path, contents).with_context(|| format!("write {}", path.display()))?;
    }

    Ok(())
}

async fn run_native_proposal(args: Args, input_path: PathBuf, input: GuestInput) -> Result<()> {
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
    let wall_time_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);

    write_proof_json(output_path, &proof)?;

    if let Some(input_hash) = proof.input {
        println!("public_values: {input_hash:#x}");
    }

    if let Some(path) = &args.json_out {
        let report = BenchReport {
            stage: "proposal",
            mode: args.mode.as_str(),
            proof_mode: args.proof_mode.as_str(),
            input: input_path.display().to_string(),
            public_values: proof
                .input
                .map(|h| format!("{h:#x}"))
                .unwrap_or_else(String::new),
            wall_time_ms,
            exit_code: None,
            gas: None,
            total_instruction_count: None,
            total_syscall_count: None,
            touched_memory_addresses: None,
            cycle_tracker: Vec::new(),
            invocation_tracker: Vec::new(),
            opcode_counts: Vec::new(),
            syscall_counts: Vec::new(),
        };
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
) -> Result<(
    ShastaZkAggregationGuestInput,
    Vec<SP1ProofWithPublicValues>,
    [u32; 8],
)> {
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
        block_inputs.push(hash_shasta_subproof_input(&carry));
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

        let proof_hex = proof.proof.as_deref().context("missing proof bytes")?;
        let proof_bytes = hex::decode(proof_hex).context("decode proof hex")?;
        let sp1_proof: SP1ProofWithPublicValues =
            bincode::deserialize(&proof_bytes).context("deserialize SP1 proof")?;
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
    let raw = uuid.strip_prefix("0x").unwrap_or(uuid);
    let bytes = hex::decode(raw).context("decode uuid hex")?;
    if bytes.len() != 32 {
        anyhow::bail!("invalid uuid length: {}", bytes.len());
    }
    let mut words = [0u32; 8];
    for (idx, word) in words.iter_mut().enumerate() {
        let start = idx * 4;
        let end = start + 4;
        *word = u32::from_be_bytes(bytes[start..end].try_into().unwrap());
    }
    Ok(words)
}

fn write_proof_json(path: &PathBuf, proof: &Proof) -> Result<()> {
    let contents = serde_json::to_string_pretty(proof).context("serialize proof json")?;
    fs::write(path, contents).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_image_id_from_uuid;

    #[test]
    fn parses_image_id_from_uuid_hex() {
        let uuid = "0x000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
        let image_id = parse_image_id_from_uuid(uuid).expect("parse image id");
        assert_eq!(
            image_id,
            [
                0x00010203, 0x04050607, 0x08090a0b, 0x0c0d0e0f, 0x10111213, 0x14151617, 0x18191a1b,
                0x1c1d1e1f
            ]
        );
    }
}
