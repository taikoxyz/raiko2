use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::{Parser, ValueEnum};
use raiko2_pipeline::forks::shasta::SP1_SHASTA_BACKEND;
use raiko2_pipeline::{ProofStage, ProverBackend};
use raiko2_primitives_shasta::GuestInput;
use raiko2_protocol_shasta::shasta::{ProofCarryData, TransitionInputData};
use sp1_sdk::utils::setup_logger;
use sp1_sdk::network::Address;
use sp1_sdk::{ProverClient, SP1ProofMode, SP1Stdin};

#[derive(Parser)]
#[command(name = "guest-launcher")]
#[command(about = "Run SP1 guest programs locally with JSON inputs", long_about = None)]
struct Args {
    /// Path to the input JSON file.
    #[arg(long)]
    input: PathBuf,
    /// Proof stage to run (proposal or aggregation).
    #[arg(long, value_enum, default_value = "proposal")]
    stage: Stage,
    /// Execution mode (execute for simulation, prove for proof generation).
    #[arg(long, value_enum, default_value = "execute")]
    mode: Mode,
    /// Proof mode when generating proofs.
    #[arg(long, value_enum, default_value = "plonk")]
    proof_mode: ProofMode,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Stage {
    Proposal,
    Aggregation,
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

    if matches!(args.stage, Stage::Aggregation) {
        bail!("aggregation stage is not implemented yet for this launcher");
    }

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

    match args.mode {
        Mode::Execute => {
            let (public_values, report) = prover.execute(elf, &stdin).run()?;
            println!("public_values: {}", public_values.raw());
            if !report.cycle_tracker.is_empty() {
                println!("cycle_tracker:");
                for (label, cycles) in report.cycle_tracker {
                    println!("  {label}: {cycles}");
                }
            }
        }
        Mode::Prove => {
            let (pk, _vk) = prover.setup(elf);
            let proof = prover
                .prove(&pk, &stdin)
                .mode(args.proof_mode.into())
                .run()
                .context("prove failed")?;
            println!("public_values: {}", proof.public_values.raw());
        }
    }

    Ok(())
}

fn read_input(path: &PathBuf) -> Result<GuestInput> {
    let contents = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let input: GuestInput = serde_json::from_str(&contents).context("parse input JSON")?;
    Ok(input)
}
