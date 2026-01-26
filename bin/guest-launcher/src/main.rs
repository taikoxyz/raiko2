use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use raiko2_pipeline::forks::shasta::SP1_SHASTA_BACKEND;
use raiko2_pipeline::{ProofStage, ProverBackend};
use raiko2_primitives_shasta::GuestInput;
use raiko2_protocol_shasta::shasta::{ProofCarryData, TransitionInputData};
use serde::Serialize;
use sp1_sdk::network::Address;
use sp1_sdk::utils::setup_logger;
use sp1_sdk::{ProverClient, SP1ProofMode, SP1Stdin};

#[derive(Parser)]
#[command(name = "guest-launcher")]
#[command(about = "Run SP1 guest programs locally with JSON inputs", long_about = None)]
struct Args {
    /// Path to the input JSON file.
    #[arg(long)]
    input: PathBuf,
    /// Execution mode (execute for simulation, prove for proof generation).
    #[arg(long, value_enum, default_value = "execute")]
    mode: Mode,
    /// Proof mode when generating proofs.
    #[arg(long, value_enum, default_value = "plonk")]
    proof_mode: ProofMode,

    /// Optional path to write a JSON benchmark report.
    #[arg(long)]
    json_out: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Mode {
    Execute,
    Prove,
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
struct BenchReport {
    stage: &'static str,
    mode: &'static str,
    proof_mode: &'static str,
    input: String,
    public_values: String,
    wall_time_ms: u64,
    cycle_tracker: Vec<BenchCycleEntry>,
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

fn main() -> Result<()> {
    // Enable SP1 runtime logs (includes guest `println!` output).
    // Use `RUST_LOG=info` (or `debug`) when running this binary to see them.
    setup_logger();

    let args = Args::parse();

    let input = read_input(&args.input)?;
    let mut stdin = SP1Stdin::new();
    // IMPORTANT: pass GuestInput as raw bincode bytes.
    // This avoids host/guest schema/config mismatches in SP1's typed IO for complex structs.
    let guest_input_bytes = bincode::serialize(&input).context("bincode serialize GuestInput")?;
    stdin.write_vec(guest_input_bytes);

    // Chain id must match what the guest expects for the witness' chain spec.
    // Prefer the witness' chain id (it is what the pipeline executes against), and fall back to
    // the manifest chain id if the witness list is empty.
    let chain_id = input
        .witnesses
        .first()
        .map(|w| w.chain_spec.chain_id)
        .filter(|&id| id != 0)
        .unwrap_or(input.taiko.chain_spec.chain_id);
    let proof_carry_data = ProofCarryData {
        chain_id,
        verifier: Address::default(),
        transition_input: TransitionInputData {
            proposal_id: input.taiko.proposal_id,
            ..Default::default()
        },
    };
    stdin.write(&proof_carry_data);

    let elf = SP1_SHASTA_BACKEND
        .elf(ProofStage::Proposal)
        .context("load SP1 proposal ELF")?;

    let prover = ProverClient::from_env();

    let mut report = BenchReport {
        stage: "proposal",
        mode: args.mode.as_str(),
        proof_mode: args.proof_mode.as_str(),
        input: args.input.display().to_string(),
        public_values: String::new(),
        wall_time_ms: 0,
        cycle_tracker: Vec::new(),
    };

    match args.mode {
        Mode::Execute => {
            let start = Instant::now();
            let (public_values, execution_report) = prover.execute(elf, &stdin).run()?;
            report.wall_time_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
            report.public_values = public_values.raw();

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
            let (pk, _vk) = prover.setup(elf);
            let start = Instant::now();
            let proof = prover
                .prove(&pk, &stdin)
                .mode(args.proof_mode.into())
                .run()
                .context("prove failed")?;
            report.wall_time_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
            report.public_values = proof.public_values.raw();
            println!("public_values: {}", report.public_values);
        }
    }

    if let Some(path) = &args.json_out {
        let contents = serde_json::to_string_pretty(&report).context("serialize bench report")?;
        fs::write(path, contents).with_context(|| format!("write {}", path.display()))?;
    }

    Ok(())
}

fn read_input(path: &PathBuf) -> Result<GuestInput> {
    let contents = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let input: GuestInput = serde_json::from_str(&contents).context("parse input JSON")?;
    Ok(input)
}
