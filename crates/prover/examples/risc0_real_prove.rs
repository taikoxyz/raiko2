#![allow(missing_docs)]

use std::{env, fs, time::Instant};

use raiko2_pipeline::{ProofStage, ProverBackend, forks::shasta::RISC0_SHASTA_BACKEND};
use raiko2_primitives::ProverConfig;
use raiko2_primitives_shasta::GuestInput;
use raiko2_prover::{
    Prover,
    risc0::{Risc0Config, Risc0Prover},
};
use risc0_zkvm::{ExecutorEnv, local_executor};

const MILLION_CYCLES: u64 = 1_000_000;
const QUOTED_MCYCLES_LIMIT: u32 = 6_000;

fn evaluated_mcycles(input: &GuestInput) -> Result<u32, Box<dyn std::error::Error>> {
    let elf = RISC0_SHASTA_BACKEND
        .elf(ProofStage::Proposal)
        .map_err(|err| format!("load risc0 proposal elf: {err}"))?
        .to_vec();
    let encoded =
        bincode::serialize(input).map_err(|err| format!("serialize guest input: {err}"))?;
    let env = ExecutorEnv::builder()
        .write_frame(encoded.as_slice())
        .build()
        .map_err(|err| format!("build executor env: {err}"))?;
    let session = local_executor()
        .execute(env, &elf)
        .map_err(|err| format!("execute risc0 proposal dry-run: {err}"))?;
    let cycles = session
        .segments
        .iter()
        .map(|segment| 1u64 << segment.po2)
        .sum::<u64>()
        .div_ceil(MILLION_CYCLES);
    Ok(u32::try_from(cycles).unwrap_or(u32::MAX))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let input_path = args.next().ok_or("usage: <input.json> <output.json>")?;
    let output_path = args.next().ok_or("usage: <input.json> <output.json>")?;

    let input_bytes = fs::read_to_string(&input_path)
        .map_err(|err| format!("read guest input from {input_path}: {err}"))?;
    let guest_input: GuestInput = serde_json::from_str(&input_bytes)
        .map_err(|err| format!("parse guest input json: {err}"))?;

    let evaluated_start = Instant::now();
    let evaluated_mcycles = evaluated_mcycles(&guest_input)?;
    let evaluated_elapsed = evaluated_start.elapsed();

    let prover = Risc0Prover::new(Risc0Config {
        bonsai: false,
        snark: false,
        mock: false,
        profile: false,
        execution_po2: 20,
        verify: true,
    });
    let prove_start = Instant::now();
    let proof = prover
        .prove(guest_input, &ProverConfig::default(), &RISC0_SHASTA_BACKEND)
        .await
        .map_err(|err| format!("generate real risc0 proof: {err}"))?;
    let prove_elapsed = prove_start.elapsed();

    let proof_bytes =
        serde_json::to_vec_pretty(&proof).map_err(|err| format!("serialize proof json: {err}"))?;
    fs::write(&output_path, proof_bytes)
        .map_err(|err| format!("write proof json to {output_path}: {err}"))?;

    println!("evaluated_mcycles={evaluated_mcycles}");
    println!("quoted_mcycles_limit={QUOTED_MCYCLES_LIMIT}");
    println!(
        "within_6000m_limit={}",
        if evaluated_mcycles <= QUOTED_MCYCLES_LIMIT {
            "true"
        } else {
            "false"
        }
    );
    println!("dry_run_elapsed_ms={}", evaluated_elapsed.as_millis());
    println!("prove_elapsed_ms={}", prove_elapsed.as_millis());
    println!(
        "proof_input={}",
        proof
            .input
            .map(|value| format!("{value:#x}"))
            .unwrap_or_default()
    );
    println!("proof_uuid={}", proof.uuid.as_deref().unwrap_or_default());
    println!(
        "proof_len={}",
        proof.proof.as_ref().map_or(0, std::string::String::len)
    );
    println!(
        "receipt_len={}",
        proof.quote.as_ref().map_or(0, std::string::String::len)
    );
    println!("proof_json={output_path}");

    Ok(())
}
