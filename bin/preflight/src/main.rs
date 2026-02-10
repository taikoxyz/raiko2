use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use raiko2_pipeline::forks::shasta::ShastaSpec;
use raiko2_pipeline::{NativeBackend, Pipeline, PipelineKey};
use raiko2_primitives::{ProofContext, ProofRequest, ProverConfig};
use raiko2_primitives_shasta::GuestInput;
use raiko2_provider::NetworkProvider;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Upstream RPC URL to fetch block/witness data.
    #[arg(long, env)]
    rpc_url: String,

    /// L2 chain ID for the proof context.
    #[arg(long)]
    l2_chain_id: u64,

    /// L1 chain ID for the proof context.
    #[arg(long, default_value_t = 1)]
    l1_chain_id: u64,

    /// Proposal ID (L1 event id) to preflight.
    #[arg(long)]
    proposal_id: u64,

    /// L2 block range start (inclusive).
    #[arg(long)]
    l2_start: u64,

    /// L2 block range end (inclusive).
    #[arg(long)]
    l2_end: u64,

    /// Proof type to record in the context (risc0 or sp1).
    #[arg(long, default_value = "sp1")]
    proof_type: String,

    /// Whether the RPC supports debug_executionWitness.
    #[arg(long, value_parser = clap::builder::BoolishValueParser::new(), default_value = "false")]
    debug_witness: bool,

    /// Optional prover address to embed in the manifest.
    #[arg(long)]
    prover: Option<String>,

    /// Optional graffiti to embed in the manifest.
    #[arg(long)]
    graffiti: Option<String>,

    /// Validate the guest input after preflight.
    #[arg(long, value_parser = clap::builder::BoolishValueParser::new(), default_value = "false")]
    validate: bool,

    /// Output path for the serialized guest input JSON.
    #[arg(short = 'o', long)]
    output: PathBuf,

    /// Pretty-print JSON output.
    #[arg(long, value_parser = clap::builder::BoolishValueParser::new(), default_value = "false")]
    pretty: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let provider =
        NetworkProvider::new(&args.rpc_url)?.with_debug_witness_support(args.debug_witness);

    if args.l2_start > args.l2_end {
        anyhow::bail!("--l2-start must be <= --l2-end");
    }

    let request = ProofRequest {
        l1_chain_id: args.l1_chain_id,
        l2_chain_id: args.l2_chain_id,
        proposal_id: args.proposal_id,
        proof_type: args.proof_type,
        blob_proof_type: None,
        prover: args.prover,
        graffiti: args.graffiti,
    };

    let mut config = ProverConfig::default();
    config["l2_block_range"] = serde_json::json!({
        "start": args.l2_start,
        "end": args.l2_end,
        "proposal_id": args.proposal_id,
    });

    let ctx = ProofContext::new(request, config);
    let spec = ShastaSpec::new(PipelineKey::ShastaNative, (), NativeBackend, provider);
    let pipeline = Pipeline::new(&spec);

    let guest_input = pipeline.build_guest_input(&ctx).await?.output;
    write_json(&args.output, &guest_input, args.pretty)?;
    Ok(())
}

fn write_json(path: &PathBuf, value: &GuestInput, pretty: bool) -> Result<()> {
    let contents = if pretty {
        serde_json::to_string_pretty(value)
    } else {
        serde_json::to_string(value)
    }
    .context("serialize guest input")?;
    fs::write(path, contents).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}
