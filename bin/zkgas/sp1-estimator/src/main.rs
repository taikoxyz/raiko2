//! Execute one opaque bincode buffer against an explicit program. Never prove or fetch inputs.
mod estimator;
mod io;
mod resources;

use std::{path::PathBuf, sync::Arc, time::Instant};

use anyhow::{Context, Result, ensure};
use clap::Parser;
use sp1_core_executor::Program;
use sp1_sdk::SP1Stdin;

const SDK_VERSION: &str = "6.3.0";
const INPUT_LIMIT: u64 = 64 * 1024 * 1024;

#[derive(Parser)]
#[command(about = "Execute-only SP1 gas estimate for one opaque bincode buffer and explicit ELF")]
struct Args {
    #[arg(long)]
    input_bincode: PathBuf,
    #[arg(long)]
    elf: PathBuf,
    #[arg(long)]
    output: PathBuf,
    /// Publish exact newly owned Linux SDK shared-memory identities before execution.
    #[arg(long)]
    resource_receipt: Option<PathBuf>,
}

fn opaque_stdin(bytes: Vec<u8>) -> SP1Stdin {
    let mut stdin = SP1Stdin::new();
    // `write(&Vec<u8>)` adds a second length prefix. The supplied payload already contains
    // the caller's bincode framing; SP1 expects it as exactly one input buffer.
    stdin.write_vec(bytes);
    stdin
}

fn main() -> Result<()> {
    let args = Args::parse();
    let input = io::read_regular(&args.input_bincode, INPUT_LIMIT)?;
    let input_sha256 = io::sha256(&input);
    let input_length = input.len();
    let elf = io::read_regular(&args.elf, INPUT_LIMIT)?;
    let executable_sha256 = io::executable_sha256()?;
    let program = Program::from(&elf)
        .map(Arc::new)
        .map_err(|error| anyhow::anyhow!("parse explicit SP1 ELF: {error:?}"))?;
    let output = io::reserve(&args.output)?;
    let resource_output = args
        .resource_receipt
        .as_deref()
        .map(io::reserve)
        .transpose()?;
    let opts = estimator::canonical_sp1_core_opts();
    let opts_report = serde_json::to_value(&opts).context("serialize executor options")?;
    let started = Instant::now();
    let (public_values, report) = estimator::execute_sp1_gas_estimator_with_opts(
        program,
        opaque_stdin(input),
        opts,
        resource_output,
    )?;
    ensure!(
        report.exit_code == 0,
        "SP1 guest exit code {}",
        report.exit_code
    );
    ensure!(report.gas().is_some(), "SP1 report has no gas estimate");
    io::write_json(
        output,
        &serde_json::json!({
            "schema_version": 1,
            "stage": "local_explicit_input_sp1_gas_estimator",
            "measurement_policy": "sp1-6.3-streaming-v1",
            "input_framing": "one opaque caller-supplied bincode buffer; schema validated externally",
            "sp1_sdk_version": SDK_VERSION,
            "sp1_core_executor_version": SDK_VERSION,
            "sp1_core_executor_runner_version": SDK_VERSION,
            "sp1_execution_engine": "gas-estimator",
            "production_sdk_parity_verified": false,
            "guest_input_sha256": input_sha256,
            "guest_input_bincode_length": input_length,
            "elf_sha256": io::sha256(&elf),
            "executable_sha256": executable_sha256,
            "source_revision": option_env!("ZKGAS_SOURCE_REVISION"),
            "manifest_sha256": io::sha256(include_bytes!("../Cargo.toml")),
            "cargo_lock_sha256": io::sha256(include_bytes!("../Cargo.lock")),
            "executor_opts": opts_report,
            "wall_time_ms": started.elapsed().as_millis(),
            "public_values": public_values.raw(),
            "prover_gas": report.gas(),
            "total_instruction_count": report.total_instruction_count(),
            "total_syscall_count": report.total_syscall_count(),
            "exit_code": report.exit_code,
            "execution_report": report,
            "execution_report_gas_units": "raw_unnormalized; prover_gas uses ExecutionReport::gas()",
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opaque_buffer_matches_sdk_serialization_without_double_prefix() {
        let value = (23u64, vec![1u8, 2, 3]);
        let bytes = bincode::serialize(&value).unwrap();
        let actual = opaque_stdin(bytes.clone());
        let mut expected = SP1Stdin::new();
        expected.write(&value);
        assert_eq!(actual.buffer, expected.buffer);
        assert_eq!(actual.buffer, vec![bytes.clone()]);
        let mut wrong = SP1Stdin::new();
        wrong.write(&bytes);
        assert_ne!(actual.buffer, wrong.buffer);
    }

    #[test]
    fn sdk_identity_matches_manifest_and_lock() {
        let manifest = include_str!("../Cargo.toml");
        let lock = include_str!("../Cargo.lock");
        for package in ["sp1-sdk", "sp1-core-executor", "sp1-core-executor-runner"] {
            assert!(manifest.contains(&format!("{package} =")));
            let entry = lock
                .split("[[package]]")
                .find(|entry| entry.contains(&format!("name = \"{package}\"")))
                .unwrap();
            assert!(entry.contains(&format!("version = \"{SDK_VERSION}\"")));
        }
        assert_eq!(manifest.matches("=6.3.0").count(), 3);
    }

    #[test]
    fn frozen_estimator_policy() {
        let opts = estimator::canonical_sp1_core_opts();
        assert_eq!(opts.memory_limit, 24 * 1024 * 1024 * 1024);
        assert_eq!(opts.gas_trace_chunk_threshold, 134_217_728);
        assert_eq!(opts.gas_trace_chunk_slots, 2);
        assert_eq!(opts.shard_size, 1 << 24);
        assert_eq!(opts.minimal_trace_chunk_threshold, 16_777_216);
        assert_eq!(opts.trace_chunk_slots, 5);
        assert_eq!(opts.sharding_threshold.element_threshold, 402_653_184);
        assert_eq!(opts.sharding_threshold.height_threshold, 4_194_304);
        assert!(!opts.global_dependencies_opt);
        assert!(!opts.recompute_gkr_trace);
    }
}
