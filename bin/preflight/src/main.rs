use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use clap::Parser;
use raiko2_pipeline::forks::shasta::ShastaSpec;
use raiko2_pipeline::{NativeBackend, Pipeline, PipelineKey};
use raiko2_primitives::{
    ChainSpec, PreflightRpcClientConfig, PreflightRpcRetryConfig, ProofContext, ProofRequest,
    ProofType, ProverConfig, ShastaRequest, SupportedChainSpecs,
};
use raiko2_primitives_shasta::{DEFAULT_GUEST_INPUT_ROOT, GuestInput, guest_input_proposal_path};
use raiko2_provider::{DEFAULT_RPC_TIMEOUT_MS, NetworkProvider, RpcClientConfig, RpcRetryConfig};

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Optional chain spec JSON file merged over the built-in chain spec list.
    #[arg(long)]
    chain_spec_file: Option<PathBuf>,

    /// L2 network key in the chain spec list.
    #[arg(long)]
    network: Option<String>,

    /// L1 network key in the chain spec list.
    #[arg(long)]
    l1_network: Option<String>,

    /// Upstream L2 RPC URL to fetch block/witness data. Overrides the selected L2 chain spec RPC.
    #[arg(long, env)]
    rpc_url: Option<String>,

    /// Optional L1 RPC URL for Shasta anchor linkage. Overrides the selected L1 chain spec RPC.
    #[arg(long, env)]
    l1_rpc_url: Option<String>,

    /// RPC client timeout in milliseconds for both L1 and L2 providers.
    #[arg(long, default_value_t = DEFAULT_RPC_TIMEOUT_MS)]
    rpc_timeout_ms: u64,

    /// Maximum number of concurrent RPC requests.
    #[arg(long, default_value_t = 32)]
    rpc_concurrency_limit: usize,

    /// Maximum retry attempts for transient RPC failures.
    #[arg(long, default_value_t = 4)]
    rpc_retry_max_attempts: u32,

    /// Initial RPC retry backoff in milliseconds.
    #[arg(long, default_value_t = 1_000)]
    rpc_retry_initial_backoff_ms: u64,

    /// RPC retry rate limit in compute units per second.
    #[arg(long, default_value_t = 1_000)]
    rpc_retry_cu_per_second: u64,

    /// L2 chain ID for the proof context. Overrides or validates `--network`.
    #[arg(long)]
    l2_chain_id: Option<u64>,

    /// L1 chain ID for the proof context. Overrides or validates `--l1-network`.
    #[arg(long)]
    l1_chain_id: Option<u64>,

    /// Proposal ID (L1 event id) to preflight.
    #[arg(long)]
    proposal_id: u64,

    /// L1 block number that contains the Shasta proposal event.
    #[arg(long)]
    l1_inclusion_block_number: u64,

    /// Last committed anchor block number carried across proposals.
    #[arg(long)]
    last_anchor_block_number: u64,

    /// L2 block range start (inclusive).
    #[arg(long)]
    l2_start: u64,

    /// L2 block range end (inclusive).
    #[arg(long)]
    l2_end: u64,

    /// Proof type to record in the context (native, risc0, sp1, sgx, or sgxgeth).
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

    /// Optional external L2 RPC used to cross-check proposal checkpoint data after preflight.
    #[arg(long)]
    verify_checkpoint_l2_rpc: Option<String>,

    /// Output path for the serialized guest input JSON.
    #[arg(short = 'o', long)]
    output: Option<PathBuf>,

    /// Save the serialized guest input into the repo-managed fixture tree.
    #[arg(long, value_parser = clap::builder::BoolishValueParser::new(), default_value = "false")]
    save_guest_input: bool,

    /// Root directory for repo-managed guest input fixtures.
    #[arg(long, default_value = DEFAULT_GUEST_INPUT_ROOT)]
    guest_input_root: PathBuf,

    /// Allow overwriting an existing repo-managed guest input fixture.
    #[arg(long, value_parser = clap::builder::BoolishValueParser::new(), default_value = "false")]
    overwrite_guest_input: bool,

    /// Pretty-print JSON output.
    #[arg(long, value_parser = clap::builder::BoolishValueParser::new(), default_value = "false")]
    pretty: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    if args.output.is_none() && !args.save_guest_input {
        anyhow::bail!("either --output or --save-guest-input is required");
    }
    let supported_chain_specs = load_supported_chain_specs(args.chain_spec_file.as_ref())?;
    let resolved = resolve_preflight_config(&args, &supported_chain_specs)?;

    let provider = NetworkProvider::new_pair_with_chain_specs_and_config(
        &resolved.l1_rpc_url,
        &resolved.l2_rpc_url,
        Some(resolved.l1_chain_spec.clone()),
        Some(resolved.l2_chain_spec.clone()),
        None,
        &RpcClientConfig {
            timeout_ms: args.rpc_timeout_ms,
            concurrency_limit: args.rpc_concurrency_limit,
            retry: RpcRetryConfig {
                max_attempts: args.rpc_retry_max_attempts,
                initial_backoff_ms: args.rpc_retry_initial_backoff_ms,
                compute_units_per_second: args.rpc_retry_cu_per_second,
            },
        },
    )?;

    if args.l2_start > args.l2_end {
        anyhow::bail!("--l2-start must be <= --l2-end");
    }

    let proof_type = args
        .proof_type
        .parse::<ProofType>()
        .map_err(anyhow::Error::msg)?;
    if args.save_guest_input && proof_type != ProofType::Native {
        anyhow::bail!("--save-guest-input requires --proof-type native");
    }

    let request = ProofRequest {
        l1_chain_id: resolved.l1_chain_id,
        l2_chain_id: resolved.l2_chain_id,
        proposal_id: args.proposal_id,
        l2_block_range: Some(raiko2_primitives::L2BlockRange {
            start: args.l2_start,
            end: args.l2_end,
        }),
        shasta: Some(ShastaRequest {
            l1_inclusion_block_number: args.l1_inclusion_block_number,
            last_anchor_block_number: args.last_anchor_block_number,
            checkpoint: None,
        }),
        proof_type,
        blob_proof_type: args.blob_proof_type,
        prover: args.prover,
        graffiti: args.graffiti,
    };

    let mut ctx = ProofContext::new(request, ProverConfig::default());
    ctx.l2_chain_spec = resolved.l2_chain_spec.to_taiko_chain_spec()?;
    ctx.preflight.l1_chain_spec = Some(resolved.l1_chain_spec.clone());
    ctx.preflight.l2_chain_spec = Some(resolved.l2_chain_spec.clone());
    ctx.preflight.rpc_client_config = Some(PreflightRpcClientConfig {
        timeout_ms: args.rpc_timeout_ms,
        concurrency_limit: args.rpc_concurrency_limit,
        retry: PreflightRpcRetryConfig {
            max_attempts: args.rpc_retry_max_attempts,
            initial_backoff_ms: args.rpc_retry_initial_backoff_ms,
            compute_units_per_second: args.rpc_retry_cu_per_second,
        },
    });
    ctx.preflight.verify_checkpoint_l2_rpc = args
        .verify_checkpoint_l2_rpc
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
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
    if args.validate {
        let validate_start = Instant::now();
        pipeline.validate(&ctx, guest_input.clone())?;
        eprintln!(
            "validate: proposal_id={} blocks={} elapsed_ms={}",
            args.proposal_id,
            guest_input.witnesses.len(),
            validate_start.elapsed().as_millis()
        );
    }
    if let Some(output) = &args.output {
        write_guest_input_json(output, &guest_input, args.pretty, true)?;
    }
    if args.save_guest_input {
        let network = args
            .network
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("--network is required with --save-guest-input"))?;
        let path = guest_input_proposal_path(
            &args.guest_input_root,
            network,
            guest_input.taiko.proposal_id,
        )?;
        write_guest_input_json(&path, &guest_input, args.pretty, args.overwrite_guest_input)?;
        eprintln!(
            "saved_guest_input: network={} proposal_id={} path={}",
            network,
            guest_input.taiko.proposal_id,
            path.display()
        );
    }
    Ok(())
}

#[derive(Debug)]
struct ResolvedPreflightConfig {
    l1_chain_spec: ChainSpec,
    l2_chain_spec: ChainSpec,
    l1_chain_id: u64,
    l2_chain_id: u64,
    l1_rpc_url: String,
    l2_rpc_url: String,
}

fn resolve_preflight_config(
    args: &Args,
    supported_chain_specs: &SupportedChainSpecs,
) -> Result<ResolvedPreflightConfig> {
    let l2_chain_spec = resolve_chain_spec(
        supported_chain_specs,
        "L2",
        "--network",
        args.network.as_deref(),
        "--l2-chain-id",
        args.l2_chain_id,
        None,
    )?;
    let l1_chain_spec = resolve_chain_spec(
        supported_chain_specs,
        "L1",
        "--l1-network",
        args.l1_network.as_deref(),
        "--l1-chain-id",
        args.l1_chain_id,
        Some(1),
    )?;
    let l2_rpc_url = resolve_rpc_url("L2", "--rpc-url", args.rpc_url.as_deref(), &l2_chain_spec)?;
    let l1_rpc_url = resolve_rpc_url(
        "L1",
        "--l1-rpc-url",
        args.l1_rpc_url.as_deref(),
        &l1_chain_spec,
    )?;

    Ok(ResolvedPreflightConfig {
        l1_chain_id: l1_chain_spec.chain_id,
        l2_chain_id: l2_chain_spec.chain_id,
        l1_chain_spec,
        l2_chain_spec,
        l1_rpc_url,
        l2_rpc_url,
    })
}

fn resolve_chain_spec(
    supported_chain_specs: &SupportedChainSpecs,
    role: &str,
    network_flag: &str,
    network: Option<&str>,
    chain_id_flag: &str,
    chain_id: Option<u64>,
    default_chain_id: Option<u64>,
) -> Result<ChainSpec> {
    if let Some(network) = network {
        let spec = supported_chain_specs
            .get_chain_spec(network)
            .with_context(|| format!("unsupported {role} network {network:?}"))?;
        if let Some(chain_id) = chain_id
            && spec.chain_id != chain_id
        {
            anyhow::bail!(
                "{chain_id_flag}={chain_id} conflicts with {network_flag}={network:?} (chain_id={})",
                spec.chain_id
            );
        }
        return Ok(spec);
    }

    let chain_id = chain_id.or(default_chain_id).ok_or_else(|| {
        anyhow::anyhow!("either {network_flag} or {chain_id_flag} is required for {role}")
    })?;

    supported_chain_specs
        .get_chain_spec_with_chain_id(chain_id)
        .with_context(|| format!("unsupported {role} chain id {chain_id}"))
}

fn resolve_rpc_url(
    role: &str,
    rpc_flag: &str,
    explicit_rpc_url: Option<&str>,
    chain_spec: &ChainSpec,
) -> Result<String> {
    if let Some(rpc_url) = explicit_rpc_url {
        if rpc_url.trim().is_empty() {
            anyhow::bail!("{rpc_flag} must not be empty");
        }
        return Ok(rpc_url.to_string());
    }

    if chain_spec.rpc.trim().is_empty() {
        anyhow::bail!(
            "{role} chain spec {:?} has no rpc URL; pass {rpc_flag}",
            chain_spec.name
        );
    }
    Ok(chain_spec.rpc.clone())
}

fn write_guest_input_json(
    path: &Path,
    value: &GuestInput,
    pretty: bool,
    overwrite: bool,
) -> Result<()> {
    if path.exists() && !overwrite {
        anyhow::bail!(
            "refusing to overwrite existing guest input: {}",
            path.display()
        );
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let contents = if pretty {
        serde_json::to_string_pretty(value)
    } else {
        serde_json::to_string(value)
    }
    .context("serialize guest input")?;
    fs::write(path, contents).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

fn load_supported_chain_specs(chain_spec_file: Option<&PathBuf>) -> Result<SupportedChainSpecs> {
    match chain_spec_file {
        Some(path) => SupportedChainSpecs::merge_from_file(path.clone()),
        None => Ok(SupportedChainSpecs::default()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args_fixture() -> Args {
        Args {
            chain_spec_file: None,
            network: Some("taiko_hoodi".to_string()),
            l1_network: Some("hoodi".to_string()),
            rpc_url: None,
            l1_rpc_url: None,
            rpc_timeout_ms: 60_000,
            rpc_concurrency_limit: 32,
            rpc_retry_max_attempts: 4,
            rpc_retry_initial_backoff_ms: 1_000,
            rpc_retry_cu_per_second: 1_000,
            verify_checkpoint_l2_rpc: None,
            l2_chain_id: None,
            l1_chain_id: None,
            proposal_id: 17_771,
            l1_inclusion_block_number: 2_674_375,
            last_anchor_block_number: 2_674_326,
            l2_start: 7_225_402,
            l2_end: 7_225_593,
            proof_type: "native".to_string(),
            prover: None,
            blob_proof_type: None,
            graffiti: None,
            validate: false,
            output: Some(PathBuf::from("/tmp/preflight.json")),
            save_guest_input: false,
            guest_input_root: PathBuf::from(DEFAULT_GUEST_INPUT_ROOT),
            overwrite_guest_input: false,
            pretty: false,
        }
    }

    #[test]
    fn resolves_networks_from_chain_specs() {
        let args = args_fixture();
        let specs = SupportedChainSpecs::default();
        let expected_l1 = specs.get_chain_spec("hoodi").expect("hoodi spec");
        let expected_l2 = specs
            .get_chain_spec("taiko_hoodi")
            .expect("taiko_hoodi spec");

        let resolved = resolve_preflight_config(&args, &specs).expect("resolve");

        assert_eq!(resolved.l1_chain_id, 560_048);
        assert_eq!(resolved.l2_chain_id, 167_013);
        assert_eq!(resolved.l1_chain_spec.name, "hoodi");
        assert_eq!(resolved.l2_chain_spec.name, "taiko_hoodi");
        assert_eq!(resolved.l1_rpc_url, expected_l1.rpc);
        assert_eq!(resolved.l2_rpc_url, expected_l2.rpc);
    }

    #[test]
    fn explicit_rpc_urls_override_chain_spec_rpcs() {
        let mut args = args_fixture();
        args.rpc_url = Some("http://l2.override".to_string());
        args.l1_rpc_url = Some("http://l1.override".to_string());
        let specs = SupportedChainSpecs::default();

        let resolved = resolve_preflight_config(&args, &specs).expect("resolve");

        assert_eq!(resolved.l1_rpc_url, "http://l1.override");
        assert_eq!(resolved.l2_rpc_url, "http://l2.override");
    }

    #[test]
    fn cli_requires_last_anchor_block_number() {
        let err = Args::try_parse_from([
            "preflight",
            "--network",
            "taiko_hoodi",
            "--l1-network",
            "hoodi",
            "--proposal-id",
            "17771",
            "--l1-inclusion-block-number",
            "2674375",
            "--l2-start",
            "7225402",
            "--l2-end",
            "7225593",
            "--output",
            "/tmp/preflight.json",
        ])
        .expect_err("missing last anchor should fail");

        assert!(
            err.to_string().contains("--last-anchor-block-number"),
            "unexpected parse error: {err}"
        );
    }

    #[test]
    fn explicit_chain_id_conflict_with_network_is_rejected() {
        let mut args = args_fixture();
        args.l2_chain_id = Some(167_000);
        let specs = SupportedChainSpecs::default();

        let err = resolve_preflight_config(&args, &specs).expect_err("reject conflict");

        assert!(err.to_string().contains("conflicts with --network"));
    }

    #[test]
    fn explicit_chain_id_mode_still_resolves_specs() {
        let mut args = args_fixture();
        args.network = None;
        args.l1_network = None;
        args.l2_chain_id = Some(167_013);
        args.l1_chain_id = Some(560_048);
        let specs = SupportedChainSpecs::default();

        let resolved = resolve_preflight_config(&args, &specs).expect("resolve");

        assert_eq!(resolved.l1_chain_spec.name, "hoodi");
        assert_eq!(resolved.l2_chain_spec.name, "taiko_hoodi");
    }

    #[test]
    fn missing_l2_network_and_chain_id_is_rejected() {
        let mut args = args_fixture();
        args.network = None;
        let specs = SupportedChainSpecs::default();

        let err = resolve_preflight_config(&args, &specs).expect_err("reject missing l2");

        assert!(
            err.to_string()
                .contains("either --network or --l2-chain-id")
        );
    }

    #[test]
    fn empty_chain_spec_rpc_requires_override() {
        let specs = SupportedChainSpecs::default();
        let mut spec = specs
            .get_chain_spec("taiko_hoodi")
            .expect("default taiko_hoodi spec");
        spec.rpc.clear();

        let err = resolve_rpc_url("L2", "--rpc-url", None, &spec).expect_err("reject empty rpc");

        assert!(err.to_string().contains("has no rpc URL"));
    }
}
