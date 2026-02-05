use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result};
use alloy_primitives::{B256, hex};
use clap::{Parser, ValueEnum};
use raiko2_pipeline::forks::shasta::SP1_SHASTA_BACKEND;
use raiko2_pipeline::{ProofStage, ProverBackend};
use raiko2_primitives::Proof;
use raiko2_primitives_shasta::GuestInput;
use raiko2_primitives_shasta::decode_proof_carry_data;
use raiko2_primitives_shasta::encode_proof_carry_data;
use raiko2_primitives_shasta::ShastaZkAggregationGuestInput;
use raiko2_primitives_shasta::instance::words_to_bytes_be;
use raiko2_protocol_shasta::libhash::hash_shasta_subproof_input;
use raiko2_protocol_shasta::shasta::{ProofCarryData, TransitionInputData};
use serde::Serialize;
use sp1_sdk::{HashableKey, SP1ProofWithPublicValues};
use sp1_sdk::network::Address;
use sp1_sdk::utils::setup_logger;
use sp1_sdk::{ProverClient, SP1ProofMode, SP1Stdin, SP1VerifyingKey};

#[derive(Parser)]
#[command(name = "guest-launcher")]
#[command(about = "Run SP1 guest programs locally with JSON inputs", long_about = None)]
struct Args {
    /// Path to the input JSON file.
    #[arg(long)]
    input: Option<PathBuf>,
    /// Proof files to aggregate.
    #[arg(long, num_args = 1..)]
    aggregate: Vec<PathBuf>,
    /// Execution mode (execute for simulation, prove for proof generation).
    #[arg(long, value_enum, default_value = "execute")]
    mode: Mode,
    /// Proof mode when generating proofs.
    #[arg(long, value_enum, default_value = "plonk")]
    proof_mode: ProofMode,
    /// Path to write proof JSON output.
    #[arg(long)]
    output: Option<PathBuf>,

    /// Optional path to write a JSON benchmark report.
    #[arg(long)]
    json_out: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, ValueEnum, PartialEq, Eq)]
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

#[derive(Debug, Serialize)]
struct BenchCycleEntry {
    label: String,
    cycles: u64,
}

#[derive(Debug, Serialize)]
struct BenchReport {
    stage: &'static str,
    mode: &'static str,
    proof_mode: &'static str,
    input: String,
    public_values: String,
    wall_time_ms: u64,
    cycle_tracker: Vec<BenchCycleEntry>,
}

impl Mode {
    const fn as_str(self) -> &'static str {
        match self {
            Mode::Execute => "execute",
            Mode::Prove => "prove",
        }
    }
}

impl ProofMode {
    const fn as_str(self) -> &'static str {
        match self {
            ProofMode::Core => "core",
            ProofMode::Compressed => "compressed",
            ProofMode::Plonk => "plonk",
        }
    }
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

    if !args.aggregate.is_empty() {
        return run_aggregation(args);
    }
    run_proposal(args)
}

fn read_input(path: &PathBuf) -> Result<GuestInput> {
    let contents = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let input: GuestInput = serde_json::from_str(&contents).context("parse input JSON")?;
    Ok(input)
}

fn run_proposal(args: Args) -> Result<()> {
    let input_path = args.input.context("missing --input")?;
    let input = read_input(&input_path)?;
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

    let mut report = BenchReport {
        stage: "proposal",
        mode: args.mode.as_str(),
        proof_mode: args.proof_mode.as_str(),
        input: input_path.display().to_string(),
        public_values: String::new(),
        wall_time_ms: 0,
        cycle_tracker: Vec::new(),
    };

    match args.mode {
        Mode::Execute => {
            let start = Instant::now();
            let (public_values, execution_report) = prover.execute(elf, &stdin).run()?;
            report.wall_time_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
            report.public_values = public_values.raw();

            println!("public_values: {}", report.public_values);
            if !execution_report.cycle_tracker.is_empty() {
                println!("cycle_tracker:");
                for (label, cycles) in execution_report.cycle_tracker {
                    println!("  {label}: {cycles}");
                    report.cycle_tracker.push(BenchCycleEntry { label, cycles });
                }
            }
        }
        Mode::Prove => {
            let (pk, vk) = prover.setup(elf);
            let start = Instant::now();
            let proof = prover
                .prove(&pk, &stdin)
                .mode(args.proof_mode.into())
                .run()
                .context("prove failed")?;
            report.wall_time_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
            report.public_values = proof.public_values.raw();
            println!("public_values: {}", report.public_values);

            if let Some(path) = &args.output {
                let output = build_sp1_proof_output(&proof, &vk, Some(&proof_carry_data))?;
                write_proof_json(path, &output)?;
            }
        }
    }

    if let Some(path) = &args.json_out {
        let contents = serde_json::to_string_pretty(&report).context("serialize bench report")?;
        fs::write(path, contents).with_context(|| format!("write {}", path.display()))?;
    }

    Ok(())
}

fn run_aggregation(args: Args) -> Result<()> {
    if args.aggregate.is_empty() {
        anyhow::bail!("missing --aggregate proofs");
    }
    let output_path = args
        .output
        .as_ref()
        .context("missing --output for aggregation")?;
    if args.mode == Mode::Execute {
        anyhow::bail!("aggregation requires --mode prove");
    }

    let proofs = read_proofs(&args.aggregate)?;
    let (aggregation_input, sp1_proofs, image_id) = build_aggregation_inputs(&proofs)?;

    let mut stdin = SP1Stdin::new();
    stdin.write(&aggregation_input);

    let proposal_elf = SP1_SHASTA_BACKEND
        .elf(ProofStage::Proposal)
        .context("load SP1 proposal ELF")?;
    let prover = ProverClient::from_env();
    let (_, proposal_vk) = prover.setup(proposal_elf);

    for proof in sp1_proofs {
        match proof.proof {
            sp1_sdk::SP1Proof::Compressed(reduce_proof) => {
                stdin.write_proof(*reduce_proof, proposal_vk.vk.clone());
            }
            _ => {
                anyhow::bail!("aggregation requires compressed proofs");
            }
        }
    }

    let elf = SP1_SHASTA_BACKEND
        .elf(ProofStage::Aggregation)
        .context("load SP1 aggregation ELF")?;
    let (pk, vk) = prover.setup(elf);
    let start = Instant::now();
    let proof = prover
        .prove(&pk, &stdin)
        .mode(args.proof_mode.into())
        .run()
        .context("prove failed")?;
    let report = BenchReport {
        stage: "aggregation",
        mode: args.mode.as_str(),
        proof_mode: args.proof_mode.as_str(),
        input: output_path.display().to_string(),
        public_values: proof.public_values.raw(),
        wall_time_ms: u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX),
        cycle_tracker: Vec::new(),
    };

    let output = build_sp1_proof_output(&proof, &vk, None)?;
    write_proof_json(output_path, &output)?;

    println!("public_values: {}", report.public_values);

    if let Some(path) = &args.json_out {
        let contents = serde_json::to_string_pretty(&report).context("serialize bench report")?;
        fs::write(path, contents).with_context(|| format!("write {}", path.display()))?;
    }

    println!("image_id: 0x{}", hex::encode(words_to_bytes_be(&image_id)));

    Ok(())
}

fn build_sp1_proof_output(
    proof: &SP1ProofWithPublicValues,
    vk: &SP1VerifyingKey,
    carry: Option<&ProofCarryData>,
) -> Result<Proof> {
    let public_values = proof.public_values.as_slice();
    let input_hash = if public_values.len() >= 32 {
        B256::from_slice(&public_values[..32])
    } else {
        B256::default()
    };
    let proof_bytes = bincode::serialize(proof).context("serialize SP1 proof")?;
    let extra_data = match carry {
        Some(data) => Some(encode_proof_carry_data(data)?),
        None => None,
    };

    Ok(Proof {
        proof: Some(hex::encode_prefixed(&proof_bytes)),
        input: Some(input_hash),
        uuid: Some(vk.bytes32()),
        extra_data,
        ..Default::default()
    })
}

fn read_proofs(paths: &[PathBuf]) -> Result<Vec<Proof>> {
    let mut proofs = Vec::with_capacity(paths.len());
    for path in paths {
        let contents = fs::read_to_string(path)
            .with_context(|| format!("read proof {}", path.display()))?;
        let proof: Proof =
            serde_json::from_str(&contents).with_context(|| format!("parse {}", path.display()))?;
        proofs.push(proof);
    }
    Ok(proofs)
}

fn build_aggregation_inputs(
    proofs: &[Proof],
) -> Result<(ShastaZkAggregationGuestInput, Vec<SP1ProofWithPublicValues>, [u32; 8])> {
    let mut proof_carry_data_vec = Vec::with_capacity(proofs.len());
    let mut block_inputs = Vec::with_capacity(proofs.len());
    let mut sp1_proofs = Vec::with_capacity(proofs.len());
    let mut image_id: Option<[u32; 8]> = None;

    for proof in proofs {
        let carry_value = proof
            .extra_data
            .as_ref()
            .context("missing proof extra_data")?;
        let carry = decode_proof_carry_data(carry_value)?;
        block_inputs.push(hash_shasta_subproof_input(&carry));
        proof_carry_data_vec.push(carry);

        let uuid = proof.uuid.as_deref().context("missing proof uuid")?;
        let candidate_id = parse_image_id_from_uuid(uuid)?;
        if let Some(existing) = image_id {
            if existing != candidate_id {
                anyhow::bail!("mismatched proof image ids");
            }
        } else {
            image_id = Some(candidate_id);
        }

        let proof_hex = proof.proof.as_deref().context("missing proof bytes")?;
        let proof_bytes = hex::decode(proof_hex).context("decode proof hex")?;
        let sp1_proof: SP1ProofWithPublicValues =
            bincode::deserialize(&proof_bytes).context("deserialize SP1 proof")?;
        sp1_proofs.push(sp1_proof);
    }

    let image_id = image_id.context("missing proof image id")?;
    let aggregation_input = ShastaZkAggregationGuestInput {
        image_id,
        block_inputs,
        proof_carry_data_vec,
        prover_address: Address::default(),
    };

    Ok((aggregation_input, sp1_proofs, image_id))
}

fn parse_image_id_from_uuid(uuid: &str) -> Result<[u32; 8]> {
    let raw = uuid.strip_prefix("0x").unwrap_or(uuid);
    let bytes = hex::decode(raw).context("decode uuid hex")?;
    if bytes.len() != 32 {
        anyhow::bail!("invalid uuid length: {}", bytes.len());
    }
    let mut words = [0u32; 8];
    for (idx, word) in words.iter_mut().enumerate() {
        let start = idx * 4;
        let end = start + 4;
        *word = u32::from_be_bytes(bytes[start..end].try_into().unwrap());
    }
    Ok(words)
}

fn write_proof_json(path: &PathBuf, proof: &Proof) -> Result<()> {
    let contents = serde_json::to_string_pretty(proof).context("serialize proof json")?;
    fs::write(path, contents).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_image_id_from_uuid;

    #[test]
    fn parses_image_id_from_uuid_hex() {
        let uuid = "0x000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
        let image_id = parse_image_id_from_uuid(uuid).expect("parse image id");
        assert_eq!(
            image_id,
            [
                0x00010203,
                0x04050607,
                0x08090a0b,
                0x0c0d0e0f,
                0x10111213,
                0x14151617,
                0x18191a1b,
                0x1c1d1e1f
            ]
        );
    }
}
