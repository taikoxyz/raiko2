#![allow(missing_docs)]

use std::{collections::BTreeMap, env, fs, path::PathBuf};

use raiko2_pipeline::{ProofStage, ProverBackend, forks::shasta::RISC0_SHASTA_BACKEND};
use raiko2_primitives_shasta::GuestInput;
use risc0_binfmt::ProgramBinary;
use risc0_zkos_v1compat::V1COMPAT_ELF;
use risc0_zkvm::{ExecutorEnv, local_executor};

const MILLION_CYCLES: u64 = 1_000_000;

#[derive(Debug, Default)]
struct Args {
    input_path: PathBuf,
    execution_po2: u32,
    elf_path: Option<PathBuf>,
    profile_out: Option<PathBuf>,
}

fn parse_args() -> Result<Args, Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let input_path = args.next().ok_or(
        "usage: cargo run -p raiko2-prover --example risc0_cyclecheck -- <input.json> [execution_po2] [--elf-path <path>] [--profile-out <path>]",
    )?;
    let mut parsed = Args {
        input_path: PathBuf::from(input_path),
        execution_po2: 20,
        elf_path: None,
        profile_out: None,
    };

    let mut first_positional_consumed = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--elf-path" => {
                let path = args
                    .next()
                    .ok_or("--elf-path requires a file path argument")?;
                parsed.elf_path = Some(PathBuf::from(path));
            }
            "--profile-out" => {
                let path = args
                    .next()
                    .ok_or("--profile-out requires a file path argument")?;
                parsed.profile_out = Some(PathBuf::from(path));
            }
            _ if !first_positional_consumed => {
                parsed.execution_po2 = arg
                    .parse::<u32>()
                    .map_err(|err| format!("parse execution_po2 from {arg}: {err}"))?;
                first_positional_consumed = true;
            }
            _ => {
                return Err(
                    "usage: cargo run -p raiko2-prover --example risc0_cyclecheck -- <input.json> [execution_po2] [--elf-path <path>] [--profile-out <path>]"
                        .into(),
                )
            }
        }
    }

    Ok(parsed)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args()?;

    let input_path = args.input_path.display().to_string();
    let input_bytes = fs::read_to_string(&args.input_path)
        .map_err(|err| format!("read guest input from {input_path}: {err}"))?;
    let guest_input: GuestInput = serde_json::from_str(&input_bytes)
        .map_err(|err| format!("parse guest input json: {err}"))?;

    let elf = if let Some(path) = &args.elf_path {
        let bytes = fs::read(path)
            .map_err(|err| format!("read risc0 elf from {}: {err}", path.display()))?;
        if ProgramBinary::decode(&bytes).is_ok() {
            bytes
        } else {
            ProgramBinary::new(&bytes, V1COMPAT_ELF).encode()
        }
    } else {
        RISC0_SHASTA_BACKEND
            .elf(ProofStage::Proposal)
            .map_err(|err| format!("load risc0 proposal elf: {err}"))?
            .to_vec()
    };
    let encoded =
        bincode::serialize(&guest_input).map_err(|err| format!("serialize guest input: {err}"))?;
    let mut env_builder = ExecutorEnv::builder();
    env_builder
        .write_frame(encoded.as_slice())
        .segment_limit_po2(args.execution_po2);
    if let Some(path) = &args.profile_out {
        env_builder.enable_profiler(path);
    }
    let env = env_builder
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
    println!("current_risc0_execution_po2={}", args.execution_po2);
    if let Some(path) = &args.elf_path {
        println!("current_risc0_elf_path={}", path.display());
    }
    if let Some(path) = &args.profile_out {
        println!("current_risc0_profile_out={}", path.display());
    }
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
