//! CPU execution experiment; never requests proving keys or creates proofs.
use anyhow::{Context, Result, ensure};
use clap::Parser;
use raiko2_primitives_shasta::{GuestInput, zisk::frame_stdin};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    path::{Path, PathBuf},
    process::Command,
    time::Instant,
};

#[derive(Parser)]
#[command(about = "Run complete Shasta validation under the ZisK CPU emulator")]
struct Args {
    #[arg(long)]
    input: PathBuf,
    #[arg(long)]
    elf: PathBuf,
    #[arg(long)]
    emulator: PathBuf,
    /// Must not already exist; artifacts from a previous run cannot be reused.
    #[arg(long)]
    output_dir: PathBuf,
    #[arg(long, default_value_t = 1_000_000_000_000)]
    max_steps: u64,
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    std::io::copy(&mut file, &mut hash)?;
    Ok(hex::encode(hash.finalize()))
}

fn main() -> Result<()> {
    let args = Args::parse();
    ensure!(args.max_steps > 0, "max-steps must be positive");
    let input_path = args.input.canonicalize().context("resolve input JSON")?;
    let elf = args.elf.canonicalize().context("resolve guest ELF")?;
    let emulator = args.emulator.canonicalize().context("resolve emulator")?;
    let source = fs::read(&input_path).context("read input JSON")?;
    let input: GuestInput =
        serde_json::from_slice(&source).context("parse complete GuestInput JSON")?;
    let native_started = Instant::now();
    let expected = raiko2_guest_common::prove_shasta_proposal(&input)
        .context("native complete proposal validation failed")?;
    let native_elapsed = native_started.elapsed().as_secs_f64();
    let payload = bincode::serialize(&input).context("serialize bincode 1.3 GuestInput")?;
    let frame = frame_stdin(&payload);
    fs::create_dir(&args.output_dir).context("create fresh output directory")?;
    let out = args.output_dir.canonicalize()?;
    fs::write(out.join("input.bin"), &frame)?;
    fs::write(out.join("expected-digest.hex"), hex::encode(expected))?;
    let version = Command::new(&emulator)
        .arg("--version")
        .output()
        .context("query emulator version")?;
    fs::write(out.join("emulator.version.stdout"), &version.stdout)?;
    fs::write(out.join("emulator.version.stderr"), &version.stderr)?;
    ensure!(version.status.success(), "emulator --version failed");
    let (costs, elapsed) =
        zisk_bench::execute_cpu(&emulator, &elf, &out, expected.as_ref(), args.max_steps)?;
    let output = fs::read(out.join("output.bin")).context("read emulator public output")?;
    let first = input
        .witnesses
        .first()
        .context("missing proposal witnesses")?;
    let last = input
        .witnesses
        .last()
        .context("missing proposal witnesses")?;
    let report = json!({
        "schema_version": 1,
        "backend": "zisk",
        "metric_kind": "zisk_emulator_operation_weighted_cost",
        "weighted_cost": costs,
        "setup_backed_cells": null,
        "sp1_prover_gas": null,
        "peak_rss_bytes": null,
        "peak_rss_source": null,
        "emulator_elapsed_seconds": elapsed,
        "native_elapsed_seconds": native_elapsed,
        "max_steps": args.max_steps,
        "input": {"path": input_path, "sha256": hex::encode(Sha256::digest(&source)), "json_bytes": source.len(), "bincode_bytes": payload.len(), "framed_bytes": frame.len()},
        "elf": {"path": elf, "sha256": sha256_file(&elf)?},
        "emulator": {"path": emulator, "sha256": sha256_file(&emulator)?, "version": String::from_utf8_lossy(&version.stdout).trim()},
        "proposal": {"chain_id": first.chain_spec.chain_id, "first_block": first.block.header.number, "last_block": last.block.header.number, "first_timestamp": first.block.header.timestamp, "last_timestamp": last.block.header.timestamp, "witness_count": input.witnesses.len()},
        "expected_digest": hex::encode(expected),
        "guest_digest": hex::encode(&output[..32]),
        "outputs_match": true,
        "proof_generated": false
    });
    fs::write(out.join("report.json"), serde_json::to_vec_pretty(&report)?)?;
    println!("{}", out.join("report.json").display());
    Ok(())
}
