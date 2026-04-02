use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use raiko2_pipeline::forks::shasta::ShastaSpec;
use raiko2_pipeline::{NativeBackend, Pipeline, PipelineKey};
use raiko2_primitives::{
    ProofContext, ProofRequest, ProofType, ProverConfig, ShastaRequest, SupportedChainSpecs,
};
use raiko2_primitives_shasta::GuestInput;
use raiko2_provider::NetworkProvider;
use std::time::Instant;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Upstream L2 RPC URL to fetch block/witness data.
    #[arg(long, env)]
    rpc_url: String,

    /// Optional L1 RPC URL for Shasta anchor linkage. Defaults to `--rpc-url`.
    #[arg(long, env)]
    l1_rpc_url: Option<String>,

    /// L2 chain ID for the proof context.
    #[arg(long)]
    l2_chain_id: u64,

    /// L1 chain ID for the proof context.
    #[arg(long, default_value_t = 1)]
    l1_chain_id: u64,

    /// Proposal ID (L1 event id) to preflight.
    #[arg(long)]
    proposal_id: u64,

    /// L1 block number that contains the Shasta proposal event.
    #[arg(long)]
    l1_inclusion_block_number: u64,

    /// Last committed anchor block number carried across proposals.
    #[arg(long, default_value_t = 0)]
    last_anchor_block_number: u64,

    /// L2 block range start (inclusive).
    #[arg(long)]
    l2_start: u64,

    /// L2 block range end (inclusive).
    #[arg(long)]
    l2_end: u64,

    /// Proof type to record in the context (risc0 or sp1).
    #[arg(long, default_value = "sp1")]
    proof_type: String,

    /// Optional prover address to embed in the manifest.
    #[arg(long)]
    prover: Option<String>,

    /// Optional blob proof strategy to record in the context.
    #[arg(long)]
    blob_proof_type: Option<String>,

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

    let l1_rpc_url = args.l1_rpc_url.as_deref().unwrap_or(&args.rpc_url);
    let provider = NetworkProvider::new_pair(l1_rpc_url, &args.rpc_url)?;

    if args.l2_start > args.l2_end {
        anyhow::bail!("--l2-start must be <= --l2-end");
    }

    let request = ProofRequest {
        l1_chain_id: args.l1_chain_id,
        l2_chain_id: args.l2_chain_id,
        proposal_id: args.proposal_id,
        l2_block_range: Some(raiko2_primitives::L2BlockRange {
            start: args.l2_start,
            end: args.l2_end,
        }),
        shasta: Some(ShastaRequest {
            l1_inclusion_block_number: args.l1_inclusion_block_number,
            last_anchor_block_number: args.last_anchor_block_number,
        }),
        proof_type: args
            .proof_type
            .parse::<ProofType>()
            .map_err(anyhow::Error::msg)?,
        blob_proof_type: args.blob_proof_type,
        prover: args.prover,
        graffiti: args.graffiti,
    };

    let mut ctx = ProofContext::new(request, ProverConfig::default());
    if let Some(l2_chain_spec) =
        SupportedChainSpecs::default().get_chain_spec_with_chain_id(args.l2_chain_id)
    {
        ctx.l2_chain_spec = l2_chain_spec.to_taiko_chain_spec()?;
    }
    let spec = ShastaSpec::new(PipelineKey::ShastaNative, (), NativeBackend, provider);
    let pipeline = Pipeline::new(&spec);

    let start = Instant::now();
    let guest_input = pipeline.build_guest_input(&ctx).await?.output;
    eprintln!(
        "preflight: proposal_id={} blocks={} elapsed_ms={}",
        args.proposal_id,
        guest_input.witnesses.len(),
        start.elapsed().as_millis()
    );
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
