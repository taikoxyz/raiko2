use anyhow::{Context, Result, bail};
use raiko2_pipeline::forks::shasta::SP1_SHASTA_BACKEND;
use raiko2_pipeline::{ProofStage, ProverBackend};
use raiko2_primitives_shasta::GuestInput;
use sp1_sdk::utils::setup_logger;
use sp1_sdk::{ProverClient, SP1ProofMode, SP1Stdin};

use crate::{Args, Mode, ProofMode, Stage};

impl From<ProofMode> for SP1ProofMode {
    fn from(value: ProofMode) -> Self {
        match value {
            ProofMode::Core => SP1ProofMode::Core,
            ProofMode::Compressed => SP1ProofMode::Compressed,
            ProofMode::Plonk => SP1ProofMode::Plonk,
        }
    }
}

pub fn run(args: &Args) -> Result<()> {
    // Enable SP1 runtime logs (includes guest `println!` output).
    // Use `RUST_LOG=info` (or `debug`) when running this binary to see them.
    setup_logger();

    if !matches!(args.stage, Stage::Proposal) {
        bail!("aggregation stage is not implemented yet for this launcher");
    }

    let input: GuestInput = crate::common::read_json_file(&args.input)?;
    let proof_carry_data = crate::common::proof_carry_data_for_guest_input(&input);

    let mut stdin = SP1Stdin::new();
    // IMPORTANT: pass GuestInput as raw bincode bytes.
    // This avoids host/guest schema/config mismatches in SP1's typed IO for complex structs.
    let guest_input_bytes = bincode::serialize(&input).context("bincode serialize GuestInput")?;
    stdin.write_vec(guest_input_bytes);
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

