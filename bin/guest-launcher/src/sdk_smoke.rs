//! Fixed, tiny SDK flow checks; deliberately separate from Taiko guest execution.

use super::{Args, CliSp1ProverMode, Mode, OpcodeLabInput, ProofType};
use alloy_primitives::{hex, keccak256};
use anyhow::{Context, Result, bail, ensure};
use clap::{ArgMatches, parser::ValueSource};
use risc0_binfmt::ProgramBinary;
use risc0_zkvm::{ExecutorEnv, ExitCode, local_executor};
use serde::{Deserialize, Serialize};
use sp1_sdk::{
    SP1Stdin,
    blocking::{LightProver, Prover},
};
use std::fs;

const SP1_INPUT: &str = include_str!("../../../tests/fixtures/cuda-sdk-smoke/sp1-add.json");
const RISC0_INPUT: &str = include_str!("../../../tests/fixtures/cuda-sdk-smoke/risc0-loop.json");
const SP1_ELF: &[u8] = include_bytes!("../../../crates/guests/elf/sp1_opcode_lab.elf");
const RISC0_ELF: &[u8] = include_bytes!("../../../tests/fixtures/cuda-sdk-smoke/risc0-loop.bin");
const SP1_JOURNAL: &str = "5122fb7425a8c48308cb691e90bf72c00d68db638f065ef886d5909adb3205df";

#[derive(Debug, Serialize)]
struct SmokeReport {
    stage: &'static str,
    backend: &'static str,
    mode: &'static str,
    accelerator: &'static str,
    elf_identity: String,
    expected_journal: String,
    actual_journal: String,
    verified: bool,
    proof_kind: Option<&'static str>,
    // Proof bytes are saved only with --output; keep stdout and --json-out compact.
    #[serde(skip_serializing)]
    proof: Option<serde_json::Value>,
}

pub(super) fn validate(args: &Args, matches: &ArgMatches) -> Result<()> {
    ensure!(
        args.proof_type != ProofType::Native,
        "sdk-smoke requires --proof-type sp1 or risc0"
    );
    ensure!(
        args.aggregate.is_empty() && args.proof_mode.is_none(),
        "sdk-smoke has fixed compressed/succinct proofs; --aggregate and --proof-mode are unsupported"
    );
    ensure!(
        args.elf.is_none() && args.input_list.is_none() && args.jsonl_out.is_none(),
        "sdk-smoke uses one fixed embedded ELF/input; --elf, --input-list and --jsonl-out are unsupported"
    );
    ensure!(
        args.sp1_prover.is_none() || args.sp1_prover == Some(CliSp1ProverMode::Local),
        "sdk-smoke does not support network or mock proving"
    );
    for id in [
        "sp1_network_mode",
        "sp1_fulfillment_strategy",
        "sp1_skip_simulation",
        "sp1_cycle_limit",
        "sp1_timeout_secs",
        "risc0_execution_po2",
    ] {
        ensure!(
            matches.value_source(id) != Some(ValueSource::CommandLine),
            "sdk-smoke does not support --{}",
            id.replace('_', "-")
        );
    }
    ensure!(
        args.mode == Mode::Prove || args.output.is_none(),
        "--output requires --mode prove; use --json-out for an execute report"
    );
    ensure!(
        args.mode != Mode::Prove || cfg!(feature = "cuda"),
        "sdk-smoke prove requires a CUDA build: cargo build -p guest-launcher --features cuda; no CPU/network fallback is allowed"
    );
    Ok(())
}

fn parse_sp1_input(json: &str) -> Result<OpcodeLabInput> {
    let input: OpcodeLabInput = serde_json::from_str(json).context("parse SP1 smoke input")?;
    let expected: OpcodeLabInput = serde_json::from_str(SP1_INPUT)?;
    ensure!(
        input == expected,
        "sdk-smoke requires the fixed sp1-add.json workload"
    );
    Ok(input)
}

fn parse_risc0_input(json: &str) -> Result<u32> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Input {
        iterations: u32,
    }
    let input: Input = serde_json::from_str(json).context("parse RISC0 smoke input")?;
    ensure!(
        input.iterations == 0,
        "sdk-smoke requires the fixed zero-iteration RISC0 input"
    );
    Ok(input.iterations)
}

fn check_journal(actual: &[u8], expected: &[u8]) -> Result<()> {
    ensure!(
        actual == expected,
        "smoke journal mismatch: expected 0x{}, got 0x{}",
        hex::encode(expected),
        hex::encode(actual)
    );
    Ok(())
}

fn report(
    backend: &'static str,
    identity: String,
    journal: &[u8],
    expected: &[u8],
) -> Result<SmokeReport> {
    check_journal(journal, expected)?;
    Ok(SmokeReport {
        stage: "sdk-smoke",
        backend,
        mode: "execute",
        accelerator: "cpu",
        elf_identity: identity,
        expected_journal: format!("0x{}", hex::encode(expected)),
        actual_journal: format!("0x{}", hex::encode(journal)),
        verified: false,
        proof_kind: None,
        proof: None,
    })
}

fn sp1_stdin(input: OpcodeLabInput) -> SP1Stdin {
    let mut stdin = SP1Stdin::new();
    stdin.write(&input);
    stdin
}

fn execute_sp1(input: OpcodeLabInput) -> Result<SmokeReport> {
    let prover = LightProver::new();
    let (values, _) = prover
        .execute(SP1_ELF.into(), sp1_stdin(input))
        .run()
        .map_err(|error| anyhow::anyhow!("SP1 smoke execute failed: {error:?}"))?;
    report(
        "sp1",
        format!("elf-keccak256:{}", keccak256(SP1_ELF)),
        values.as_slice(),
        &hex::decode(SP1_JOURNAL)?,
    )
}

fn risc0_binary() -> Vec<u8> {
    ProgramBinary::new(RISC0_ELF, risc0_zkos_v1compat::V1COMPAT_ELF).encode()
}

fn risc0_env(iterations: u32) -> Result<ExecutorEnv<'static>> {
    ExecutorEnv::builder()
        .write_slice(&iterations.to_le_bytes())
        .build()
}

fn execute_risc0(iterations: u32) -> Result<SmokeReport> {
    let binary = risc0_binary();
    let session = local_executor().execute(risc0_env(iterations)?, &binary)?;
    ensure!(
        session.exit_code == ExitCode::Halted(0),
        "unexpected RISC0 smoke exit: {:?}",
        session.exit_code
    );
    report(
        "risc0",
        format!("image-id:{}", risc0_zkvm::compute_image_id(&binary)?),
        &session.journal.bytes,
        &[],
    )
}

#[cfg(feature = "cuda")]
fn prove_sp1(input: OpcodeLabInput) -> Result<SmokeReport> {
    use sp1_sdk::{
        ProvingKey, SP1Proof,
        blocking::{ProveRequest, ProverClient},
    };
    // Explicit CUDA client: never read SP1_PROVER or select CPU/network/mock proving.
    raiko2_prover::sp1::validate_cuda_server_binary().map_err(anyhow::Error::msg)?;
    let prover = ProverClient::builder().cuda().build();
    let key = prover
        .setup(SP1_ELF.into())
        .map_err(|error| anyhow::anyhow!("SP1 CUDA setup failed: {error}"))?;
    let proof = prover
        .prove(&key, sp1_stdin(input))
        .compressed()
        .run()
        .map_err(|error| anyhow::anyhow!("SP1 CUDA proof failed: {error}"))?;
    ensure!(
        matches!(&proof.proof, SP1Proof::Compressed(_)),
        "expected compressed SP1 proof"
    );
    prover
        .verify(&proof, key.verifying_key(), None)
        .context("verify SP1 smoke proof")?;
    let mut result = report(
        "sp1",
        format!("elf-keccak256:{}", keccak256(SP1_ELF)),
        proof.public_values.as_slice(),
        &hex::decode(SP1_JOURNAL)?,
    )?;
    result.mode = "prove";
    result.accelerator = "cuda";
    result.verified = true;
    result.proof_kind = Some("compressed");
    result.proof = Some(serde_json::to_value(proof)?);
    Ok(result)
}

#[cfg(any(feature = "cuda", test))]
fn verify_risc0(receipt: &risc0_zkvm::Receipt, image_id: risc0_zkvm::Digest) -> Result<()> {
    ensure!(
        matches!(&receipt.inner, risc0_zkvm::InnerReceipt::Succinct(_)),
        "expected genuine succinct RISC0 receipt; fake receipts are forbidden"
    );
    ensure!(
        receipt.claim()?.as_value()?.exit_code == ExitCode::Halted(0),
        "unexpected RISC0 smoke proof exit"
    );
    receipt
        .verify_with_context(
            &risc0_zkvm::VerifierContext::default().with_dev_mode(false),
            image_id,
        )
        .context("verify genuine RISC0 smoke receipt")?;
    check_journal(&receipt.journal.bytes, &[])
}

#[cfg(feature = "cuda")]
fn prove_risc0(iterations: u32) -> Result<SmokeReport> {
    let binary = risc0_binary();
    let image_id = risc0_zkvm::compute_image_id(&binary)?;
    let opts = risc0_zkvm::ProverOpts::succinct().with_dev_mode(false);
    let context = risc0_zkvm::VerifierContext::default().with_dev_mode(false);
    // In this exact SDK, CUDA features select CUDA segment and recursion HALs at compile time.
    // Calling the local server directly bypasses RISC0_PROVER and Bonsai selection.
    let receipt = risc0_zkvm::get_prover_server(&opts)?
        .prove_with_ctx(risc0_env(iterations)?, &context, &binary)?
        .receipt;
    verify_risc0(&receipt, image_id)?;
    let mut result = report(
        "risc0",
        format!("image-id:{image_id}"),
        &receipt.journal.bytes,
        &[],
    )?;
    result.mode = "prove";
    result.accelerator = "cuda";
    result.verified = true;
    result.proof_kind = Some("succinct");
    result.proof = Some(serde_json::to_value(receipt)?);
    Ok(result)
}

pub(super) fn run(args: Args) -> Result<()> {
    // This guard also protects future callers that bypass the CLI validation.
    ensure!(
        args.mode != Mode::Prove || cfg!(feature = "cuda"),
        "sdk-smoke prove requires --features cuda; no CPU/network fallback is allowed"
    );
    let json = args
        .input
        .as_ref()
        .map(fs::read_to_string)
        .transpose()
        .context("read smoke input")?;
    let result = match (args.proof_type, args.mode) {
        (ProofType::Sp1, Mode::Execute) => {
            execute_sp1(parse_sp1_input(json.as_deref().unwrap_or(SP1_INPUT))?)?
        }
        (ProofType::Risc0, Mode::Execute) => {
            execute_risc0(parse_risc0_input(json.as_deref().unwrap_or(RISC0_INPUT))?)?
        }
        #[cfg(feature = "cuda")]
        (ProofType::Sp1, Mode::Prove) => {
            prove_sp1(parse_sp1_input(json.as_deref().unwrap_or(SP1_INPUT))?)?
        }
        #[cfg(feature = "cuda")]
        (ProofType::Risc0, Mode::Prove) => {
            prove_risc0(parse_risc0_input(json.as_deref().unwrap_or(RISC0_INPUT))?)?
        }
        _ => bail!("unsupported SDK smoke backend/mode"),
    };
    if let Some(path) = args.output {
        fs::write(
            path,
            serde_json::to_vec_pretty(result.proof.as_ref().context("no proof generated")?)?,
        )?;
    }
    let json = serde_json::to_string_pretty(&result)?;
    if let Some(path) = args.json_out {
        fs::write(path, &json)?;
    }
    println!("{json}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, FromArgMatches};

    fn args(extra: &[&str]) -> (Args, clap::ArgMatches) {
        let mut argv = vec![
            "guest-launcher",
            "--stage",
            "sdk-smoke",
            "--proof-type",
            "sp1",
        ];
        argv.extend(extra);
        let matches = Args::command().try_get_matches_from(argv).unwrap();
        (Args::from_arg_matches(&matches).unwrap(), matches)
    }

    #[test]
    fn rejects_network_mock_and_incompatible_options_before_execution() {
        for extra in [
            vec!["--sp1-prover", "network"],
            vec!["--sp1-prover", "mock"],
            vec!["--aggregate", "missing.json"],
            vec!["--proof-mode", "compressed"],
            vec!["--elf", "missing.elf"],
            vec!["--sp1-network-mode", "reserved"],
        ] {
            let (args, matches) = args(&extra);
            assert!(validate(&args, &matches).is_err(), "accepted {extra:?}");
        }
        let (mut args, matches) = args(&[]);
        args.proof_type = ProofType::Native;
        assert!(validate(&args, &matches).is_err());
    }

    #[test]
    fn accepts_fixed_inputs_and_rejects_changed_workload() {
        let input = parse_sp1_input(SP1_INPUT).unwrap();
        assert_eq!(input.bytecode, [0x60, 1, 0x60, 2, 1, 0]);
        assert!(
            parse_sp1_input(&SP1_INPUT.replace("\"target_count\":1", "\"target_count\":2"))
                .is_err()
        );
        assert_eq!(parse_risc0_input(RISC0_INPUT).unwrap(), 0);
        assert!(parse_risc0_input("{\"iterations\":1}").is_err());
        assert!(parse_risc0_input("{}").is_err());
    }

    #[test]
    fn exact_journal_check_rejects_wrong_bytes_and_length() {
        assert!(check_journal(&[1, 2], &[1, 2]).is_ok());
        assert!(check_journal(&[1, 3], &[1, 2]).is_err());
        assert!(check_journal(&[1, 2, 0], &[1, 2]).is_err());
    }

    #[cfg(not(feature = "cuda"))]
    #[test]
    fn prove_without_cuda_fails_before_reading_input_or_selecting_cpu() {
        let (args, matches) = args(&["--mode", "prove", "--input", "missing.json"]);
        let error = validate(&args, &matches).unwrap_err().to_string();
        assert!(error.contains("--features cuda"), "{error}");
    }

    #[test]
    fn rejects_risc0_fake_receipt_even_when_it_claims_expected_output() {
        use risc0_zkvm::{FakeReceipt, InnerReceipt, Receipt, ReceiptClaim};
        let image_id = risc0_zkvm::Digest::from([0u32; 8]);
        let receipt = Receipt::new(
            InnerReceipt::Fake(FakeReceipt::new(ReceiptClaim::ok(image_id, vec![]))),
            vec![],
        );
        assert!(verify_risc0(&receipt, image_id).is_err());
    }

    #[test]
    fn cpu_executes_sp1_fixture_with_expected_public_values() {
        let report = execute_sp1(parse_sp1_input(SP1_INPUT).unwrap()).unwrap();
        assert_eq!(
            report.actual_journal,
            "0x5122fb7425a8c48308cb691e90bf72c00d68db638f065ef886d5909adb3205df"
        );
        assert_eq!(report.accelerator, "cpu");
        assert!(!report.verified);
    }

    #[test]
    fn cpu_executes_risc0_fixture_with_empty_journal() {
        let report = execute_risc0(0).unwrap();
        assert_eq!(report.actual_journal, "0x");
        assert_eq!(report.accelerator, "cpu");
        assert!(!report.verified);
    }
}
