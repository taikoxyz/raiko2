use anyhow::{Context, Result, bail};
use raiko2_pipeline::forks::shasta::RISC0_SHASTA_BACKEND;
use raiko2_pipeline::{ProofStage, ProverBackend};
use raiko2_primitives_shasta::GuestInput;
use risc0_zkvm::{ExecutorEnv, ProverOpts, compute_image_id, default_prover};

use crate::{Args, Mode, ProofMode, Stage};

pub fn run(args: &Args) -> Result<()> {
    if !matches!(args.stage, Stage::Proposal) {
        bail!("aggregation stage is not implemented yet for this launcher");
    }

    let input: GuestInput = crate::common::read_json_file(&args.input)?;
    let proof_carry_data = crate::common::proof_carry_data_for_guest_input(&input);

    let env = ExecutorEnv::builder()
        .write(&input)
        .context("write GuestInput")?
        .write(&proof_carry_data)
        .context("write ProofCarryData")?
        .build()
        .context("build risc0 ExecutorEnv")?;

    let elf = RISC0_SHASTA_BACKEND
        .elf(ProofStage::Proposal)
        .context("load RISC0 proposal ELF")?;

    let prover = default_prover();

    match args.mode {
        Mode::Execute => {
            // The `default_prover()` in our build returns a trait object that doesn't expose
            // `execute()`. For the launcher, we treat "execute" as "prove with default opts"
            // but skip verification.
            let prove_info = prover
                .prove_with_opts(env, elf, &ProverOpts::default())
                .context("execute (prove_with_opts) failed")?;
            println!(
                "journal: 0x{}",
                hex::encode(prove_info.receipt.journal.bytes)
            );
        }
        Mode::Prove => {
            let image_id = compute_image_id(elf).context("compute image id")?;
            let opts = match args.proof_mode {
                // Treat "plonk" as "produce a SNARK" for the launcher UX.
                ProofMode::Plonk => ProverOpts::groth16(),
                ProofMode::Core | ProofMode::Compressed => ProverOpts::default(),
            };
            let prove_info = prover
                .prove_with_opts(env, elf, &opts)
                .context("prove failed")?;
            prove_info
                .receipt
                .verify(image_id)
                .context("receipt verify failed")?;
            println!(
                "journal: 0x{}",
                hex::encode(prove_info.receipt.journal.bytes)
            );
        }
    }

    Ok(())
}

