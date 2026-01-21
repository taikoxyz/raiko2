use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, ValueEnum};

mod common;
mod risc0_launcher;
mod sp1_launcher;

#[derive(Parser, Debug)]
#[command(name = "guest-launcher")]
#[command(about = "Run Shasta guest programs locally with JSON inputs", long_about = None)]
struct Args {
    /// zkVM backend to use.
    #[arg(long, value_enum, default_value = "sp1")]
    backend: Backend,
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
enum Backend {
    Sp1,
    Risc0,
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

fn main() -> Result<()> {
    let args = Args::parse();
    match args.backend {
        Backend::Sp1 => sp1_launcher::run(&args),
        Backend::Risc0 => risc0_launcher::run(&args),
    }
}

