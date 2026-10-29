//! Feature-gated, offline ProgramRequest producer for native workload experiments.
mod constructor;
mod observer;
mod storage;
pub use observer::OpcodeObservation;
pub use storage::{ProgramStorageState, ProgramStorageStateReport};

use crate::{
    identity::sha256_hex,
    output::{AtomicFileWriter, CommitOutcome},
};
use anyhow::{Context, Result, bail, ensure};
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
};

const MAX_REQUEST_BYTES: u64 = 4 * 1024 * 1024;
const MAX_OUTPUT_BYTES: u64 = 128 * 1024 * 1024;
fn one() -> usize {
    1
}
fn yes() -> bool {
    true
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramRequest {
    pub schema_version: u32,
    #[serde(default = "yes")]
    pub expected_user_success: bool,
    pub program: Program,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Program {
    pub code_hex: String,
    pub input_hex: String,
    pub expected_route_counts: BTreeMap<String, u64>,
    #[serde(default = "one")]
    pub block_count: usize,
    #[serde(default)]
    pub storage_state: Option<ProgramStorageState>,
}
fn program_bytes(value: &str) -> Result<Vec<u8>> {
    hex::decode(
        value
            .strip_prefix("0x")
            .context("program bytes require a 0x prefix")?,
    )
    .context("decode program bytes")
}
pub fn parse_request(bytes: &[u8]) -> Result<ProgramRequest> {
    ensure!(
        bytes.len() as u64 <= MAX_REQUEST_BYTES,
        "request byte limit exceeded"
    );
    let request: ProgramRequest = serde_json::from_slice(bytes).context("parse ProgramRequest")?;
    ensure!(
        request.schema_version == 1,
        "fixture request schema_version must be 1"
    );
    let program = &request.program;
    ensure!(
        (1..=32).contains(&program.block_count),
        "local program block_count must be in 1..=32"
    );
    let code = program_bytes(&program.code_hex)?;
    let input = program_bytes(&program.input_hex)?;
    ensure!(
        !code.is_empty() && code.len() <= 24_576 && input.len() <= 1_048_576,
        "local program must contain 1..=24576 code bytes and at most 1MiB calldata"
    );
    ensure!(
        !program.expected_route_counts.is_empty() && program.expected_route_counts.len() <= 256,
        "program requires a bounded nonempty expected_route_counts map"
    );
    for (route, count) in &program.expected_route_counts {
        let opcode = route
            .strip_prefix("opcode:0x")
            .filter(|digits| digits.len() == 2)
            .and_then(|digits| u8::from_str_radix(digits, 16).ok())
            .context("expected routes must be canonical opcode:0xNN keys")?;
        ensure!(
            route == &format!("opcode:0x{opcode:02x}")
                && *count <= 15_000_000 * program.block_count as u64,
            "program expected route or count exceeds local bounds"
        );
    }
    if let Some(storage) = &program.storage_state {
        storage.validate(program.block_count)?;
    }
    Ok(request)
}

#[derive(Debug, Parser)]
#[command(about = "Export a bounded offline ProgramRequest as canonical Shasta GuestInput")]
pub struct FixtureArgs {
    #[arg(long)]
    pub input: PathBuf,
    #[arg(long)]
    pub output: PathBuf,
    #[arg(long)]
    pub output_bincode: Option<PathBuf>,
    #[arg(long)]
    pub report: Option<PathBuf>,
}
fn read_request(path: &Path) -> Result<Vec<u8>> {
    use rustix::fs::{CWD, Mode, OFlags, openat};
    let file = File::from(openat(
        CWD,
        path,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
        Mode::empty(),
    )?);
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file() && metadata.len() <= MAX_REQUEST_BYTES,
        "request must be a regular file of at most4MiB"
    );
    let mut bytes = Vec::new();
    file.take(MAX_REQUEST_BYTES + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_REQUEST_BYTES,
        "request byte limit exceeded"
    );
    Ok(bytes)
}
fn fresh_destinations(args: &FixtureArgs) -> Result<()> {
    let mut paths = std::collections::BTreeSet::new();
    for path in std::iter::once(&args.output)
        .chain(args.output_bincode.iter())
        .chain(args.report.iter())
    {
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let normalized = parent
            .canonicalize()?
            .join(path.file_name().context("output requires a filename")?);
        ensure!(
            paths.insert(normalized),
            "output destinations must be distinct"
        );
        match std::fs::symlink_metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
            Ok(_) => bail!("output must be a new file: {}", path.display()),
        }
    }
    Ok(())
}
struct JsonWriter<'a>(&'a mut AtomicFileWriter);
impl Write for JsonWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.append(bytes).map_err(std::io::Error::other)?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn stage_json(path: &Path, value: &impl Serialize) -> Result<AtomicFileWriter> {
    let mut output = AtomicFileWriter::new(path, MAX_OUTPUT_BYTES)?;
    serde_json::to_writer(JsonWriter(&mut output), value)?;
    output.append(b"\n")?;
    Ok(output)
}
/// Produces validated input and immutable output artifacts; never reads RPCs or credentials.
pub fn run(args: &FixtureArgs) -> Result<serde_json::Value> {
    let bytes = read_request(&args.input)?;
    let request = parse_request(&bytes)?;
    fresh_destinations(args)?;
    let built = constructor::build_program(
        &program_bytes(&request.program.code_hex)?,
        &program_bytes(&request.program.input_hex)?,
        request.program.block_count,
        request.program.storage_state.as_ref(),
    )?;
    ensure!(
        built.receipts.len() == request.program.block_count
            && built.receipts.iter().all(|receipts| receipts.len() == 2
                && receipts[0].success
                && receipts[1].success == request.expected_user_success),
        "each native block must contain one successful Anchor and one user transaction matching expected_user_success"
    );
    let mut observed = BTreeMap::<String, u64>::new();
    let mut attempted = BTreeMap::<String, u64>::new();
    let mut terminals = BTreeMap::<String, u64>::new();
    for block in &built.observations {
        for (key, count) in &block.completed {
            *observed.entry(key.clone()).or_default() += count;
        }
        for (key, count) in &block.attempted {
            *attempted.entry(key.clone()).or_default() += count;
        }
        for (key, count) in &block.terminals {
            *terminals.entry(key.clone()).or_default() += count;
        }
    }
    for (route, expected) in &request.program.expected_route_counts {
        let actual = observed.get(route).copied().unwrap_or(0);
        ensure!(
            actual == *expected,
            "native route event count differs for {route}: expected {expected}, observed {actual}"
        );
    }
    // Complete production proposal replay validates blob decoding, derivation and output identity.
    let quote = raiko2_zkgas_trace::quote_shasta_proposal(&built.input)?;
    let length = bincode::serialized_size(&built.input)?;
    ensure!(
        length <= MAX_OUTPUT_BYTES,
        "canonical GuestInput output exceeds byte limit"
    );
    let canonical = bincode::serialize(&built.input).context("serialize canonical GuestInput")?;
    let hash = sha256_hex(&canonical);
    let blocks=built.input.witnesses.iter().enumerate().map(|(index,witness)|serde_json::json!({
        "block_index":index,"block_number":witness.block.header.number,"block_timestamp":witness.block.header.timestamp,
        "block_gas_used":witness.block.header.gas_used,"block_base_fee_per_gas":witness.block.header.base_fee_per_gas,
        "transaction_count":built.receipts[index].len(),"user_transaction_gas_used":built.receipts[index][1].gas_used,
        "parent_hash":witness.block.header.parent_hash,"state_root":witness.block.header.state_root,
    })).collect::<Vec<_>>();
    let report = serde_json::json!({
        "schema_version":1,"construction_request_sha256":sha256_hex(&bytes),"guest_input_sha256":hash,
        "guest_input_bincode_length":length,"public_output":quote.public_output,"public_values":quote.public_output.as_slice(),
        "chain_id":built.input.taiko.chain_spec.chain_id,"fork_profile":"controlled_mainnet_post_unzen",
        "anchor_artifact_sha256":sha256_hex(include_bytes!("../../../../scripts/regression/shasta/Anchor.json")),
        "block_count":request.program.block_count,"blocks":blocks,"proposal_id":built.input.taiko.proposal_id,
        "transaction_count":request.program.block_count*2,"expected_user_success":request.expected_user_success,
        "expected_route_counts":request.program.expected_route_counts,"observed_route_counts":observed,
        "attempted_route_counts":attempted,"terminal_counts":terminals,"observations_by_block":built.observations,
        "receipts_by_block":built.receipts,"storage_state":built.storage,
        "user_transaction_gas_used_total":built.receipts.iter().map(|block|block[1].gas_used).sum::<u64>(),
        "gas_source":"native execution receipt cumulative_gas_used deltas (charged gas including intrinsic/floor/refund effects)",
        "opcode_count_policy":"completed step_end instructions including REVERT and reverted child work; exceptional instructions reported separately",
    });
    let mut outputs = vec![stage_json(&args.output, &built.input)?];
    if let Some(path) = &args.output_bincode {
        let mut file = AtomicFileWriter::new(path, MAX_OUTPUT_BYTES)?;
        file.append(&canonical)?;
        outputs.push(file);
    }
    if let Some(path) = &args.report {
        outputs.push(stage_json(path, &report)?);
    }
    // Recheck before publication; individual publications use atomic NOREPLACE as the final gate.
    fresh_destinations(args)?;
    for output in outputs {
        ensure!(
            output.commit()? == CommitOutcome::Created,
            "output must be a new file"
        );
    }
    Ok(
        serde_json::json!({"schema_version":1,"guest_input_sha256":hash,"guest_input_bincode_length":length,"public_output":quote.public_output}),
    )
}
