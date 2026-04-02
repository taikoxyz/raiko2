#![allow(missing_docs)]

use std::{collections::BTreeMap, env, fs};

use raiko2_pipeline::{ProofStage, ProverBackend, forks::shasta::RISC0_SHASTA_BACKEND};
use raiko2_primitives_shasta::GuestInput;
use risc0_zkvm::{ExecutorEnv, local_executor};

const MILLION_CYCLES: u64 = 1_000_000;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let input_path = args
        .next()
        .ok_or("usage: cargo run -p raiko2-prover --example risc0_cyclecheck -- <input.json> [execution_po2]")?;
    let execution_po2 = args
        .next()
        .map(|value| {
            value
                .parse::<u32>()
                .map_err(|err| format!("parse execution_po2 from {value}: {err}"))
        })
        .transpose()?
        .unwrap_or(20);
    if args.next().is_some() {
        return Err(
            "usage: cargo run -p raiko2-prover --example risc0_cyclecheck -- <input.json> [execution_po2]"
                .into(),
        );
    }

    let input_bytes = fs::read_to_string(&input_path)
        .map_err(|err| format!("read guest input from {input_path}: {err}"))?;
    let guest_input: GuestInput = serde_json::from_str(&input_bytes)
        .map_err(|err| format!("parse guest input json: {err}"))?;

    let elf = RISC0_SHASTA_BACKEND
        .elf(ProofStage::Proposal)
        .map_err(|err| format!("load risc0 proposal elf: {err}"))?
        .to_vec();
    let encoded =
        bincode::serialize(&guest_input).map_err(|err| format!("serialize guest input: {err}"))?;
    let env = ExecutorEnv::builder()
        .write_frame(encoded.as_slice())
        .segment_limit_po2(execution_po2)
        .build()
        .map_err(|err| format!("build executor env: {err}"))?;
    let session = local_executor()
        .execute(env, &elf)
        .map_err(|err| format!("execute risc0 proposal dry-run: {err}"))?;

    let padded_cycles = session
        .segments
        .iter()
        .map(|segment| 1u64 << segment.po2)
        .sum::<u64>();
    let user_cycles = session.cycles();
    let mut po2_histogram = BTreeMap::<u32, usize>::new();
    for segment in &session.segments {
        *po2_histogram.entry(segment.po2).or_default() += 1;
    }

    println!("current_input_bincode_len={}", encoded.len());
    println!("current_risc0_execution_po2={execution_po2}");
    println!("current_risc0_segments={}", session.segments.len());
    println!("current_risc0_padded_cycles={padded_cycles}");
    println!(
        "current_risc0_padded_mcycles={}",
        padded_cycles.div_ceil(MILLION_CYCLES)
    );
    println!("current_risc0_user_cycles={user_cycles}");
    println!(
        "current_risc0_user_mcycles={}",
        user_cycles.div_ceil(MILLION_CYCLES)
    );
    for (po2, count) in po2_histogram {
        println!("current_risc0_po2_count po2={po2} count={count}");
    }

    Ok(())
}
