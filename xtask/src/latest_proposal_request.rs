use std::path::{Path, PathBuf};

use alloy::{
    eips::BlockNumberOrTag,
    primitives::Log as PrimitiveLog,
    providers::{Provider as AlloyProvider, ProviderBuilder},
};
use alloy_rpc_types_eth::{Block as AlloyBlock, Filter, Log as RpcLog};
use alloy_sol_types::SolEvent;
use anyhow::{Context, Result, bail};
use clap::{Args, ValueEnum};
use raiko2_primitives::{ChainSpec, SupportedChainSpecs};
use raiko2_protocol_shasta::shasta::Proposed;
use raiko2_provider::rpc::{RpcClientConfig, RpcRetryConfig, build_rpc_client};
use reth_ethereum_primitives::Block as RethBlock;
use serde::Serialize;

const BLOCK_BATCH_SIZE: usize = 32;
const DEFAULT_ETHEREUM_RPC_URL: &str = "https://ethereum-rpc.publicnode.com";
const DEFAULT_HOODI_RPC_URL: &str = "https://ethereum-hoodi-rpc.publicnode.com";
const DEFAULT_HOODI_TAIKO_RPC_URL: &str = "http://34.172.70.130:8545";
const DEFAULT_L1_SCAN_PAGE_SIZE: u64 = 2_048;
const DEFAULT_L1_MAX_LOOKBACK: u64 = 100_000;
const DEFAULT_L2_SCAN_PAGE_SIZE: u64 = 256;
const DEFAULT_L2_MAX_LOOKBACK: u64 = 20_000;

#[derive(Args, Debug)]
pub(crate) struct LatestProposalRequestArgs {
    /// Built-in network profile.
    #[arg(long, value_enum, default_value_t = LatestProposalProfile::TaikoMainnet)]
    profile: LatestProposalProfile,

    /// Optional chain spec JSON file merged over the built-in chain spec list.
    #[arg(long)]
    chain_spec_file: Option<PathBuf>,

    /// L1 RPC URL override.
    #[arg(long)]
    l1_rpc_url: Option<String>,

    /// L2 RPC URL override.
    #[arg(long)]
    l2_rpc_url: Option<String>,

    /// Proof type to record in the request.
    #[arg(long, value_enum, default_value_t = RequestProofType::Sp1)]
    proof_type: RequestProofType,

    /// Blob proof type to include in the request body.
    #[arg(long, default_value = "proof_of_equivalence")]
    blob_proof_type: String,

    /// Emit an aggregate request instead of proposal-only.
    #[arg(long, default_value_t = false)]
    aggregate: bool,

    /// Last anchor block number carried across proposals.
    #[arg(long, default_value_t = 0)]
    last_anchor_block_number: u64,

    /// Output path for the generated JSON request. Prints to stdout when omitted.
    #[arg(short = 'o', long)]
    output: Option<PathBuf>,

    /// Maximum number of L1 blocks to scan backwards for the latest proposal event.
    #[arg(long, default_value_t = DEFAULT_L1_MAX_LOOKBACK)]
    l1_max_lookback: u64,

    /// Number of L1 blocks to query per log page while scanning backwards.
    #[arg(long, default_value_t = DEFAULT_L1_SCAN_PAGE_SIZE)]
    l1_scan_page_size: u64,

    /// Maximum number of L2 blocks to scan backwards for the proposal range.
    #[arg(long, default_value_t = DEFAULT_L2_MAX_LOOKBACK)]
    l2_max_lookback: u64,

    /// Number of L2 blocks to query per page while scanning backwards.
    #[arg(long, default_value_t = DEFAULT_L2_SCAN_PAGE_SIZE)]
    l2_scan_page_size: u64,

    /// RPC client timeout in milliseconds.
    #[arg(long, default_value_t = 60_000)]
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
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum LatestProposalProfile {
    TaikoMainnet,
    TaikoHoodi,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum RequestProofType {
    Native,
    Sp1,
    Risc0,
    ZkAny,
}

#[derive(Debug, Clone)]
struct ResolvedProfile {
    network: &'static str,
    l1_network: &'static str,
    l1_rpc_url: String,
    l2_rpc_url: String,
    chain_spec: ChainSpec,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LatestProposalRef {
    proposal_id: u64,
    l1_inclusion_block_number: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct L2ProposalRange {
    start: u64,
    end: u64,
}

#[derive(Debug, Default)]
struct DescendingRangeFinder {
    target_proposal_id: u64,
    start: Option<u64>,
    end: Option<u64>,
    closed: bool,
}

#[derive(Debug, Serialize)]
struct BatchRequestFile {
    proposals: Vec<ProposalRequestFile>,
    aggregate: bool,
    proof_type: &'static str,
    network: &'static str,
    l1_network: &'static str,
    blob_proof_type: String,
}

#[derive(Debug, Serialize)]
struct ProposalRequestFile {
    proposal_id: u64,
    l1_inclusion_block_number: u64,
    l2_block_numbers: Vec<u64>,
    last_anchor_block_number: u64,
}

pub(crate) async fn run(root: &Path, args: LatestProposalRequestArgs) -> Result<()> {
    validate_args(&args)?;
    let chain_specs = load_supported_chain_specs(args.chain_spec_file.as_ref())?;
    let profile = resolve_profile(&chain_specs, &args)?;
    let rpc_config = RpcClientConfig {
        timeout_ms: args.rpc_timeout_ms,
        concurrency_limit: args.rpc_concurrency_limit,
        retry: RpcRetryConfig {
            max_attempts: args.rpc_retry_max_attempts,
            initial_backoff_ms: args.rpc_retry_initial_backoff_ms,
            compute_units_per_second: args.rpc_retry_cu_per_second,
        },
    };

    let l1_client = build_rpc_client(&profile.l1_rpc_url, &rpc_config)
        .with_context(|| format!("build L1 RPC client for {}", profile.l1_rpc_url))?;
    let l2_client = build_rpc_client(&profile.l2_rpc_url, &rpc_config)
        .with_context(|| format!("build L2 RPC client for {}", profile.l2_rpc_url))?;
    let l1_provider = ProviderBuilder::new().connect_client(l1_client.clone());
    let l2_provider = ProviderBuilder::new().connect_client(l2_client.clone());

    let latest_l2_block_number = l2_provider
        .get_block_number()
        .await
        .context("fetch latest L2 block number")?;
    let latest_l2_block = fetch_blocks(&l2_client, &[latest_l2_block_number])
        .await?
        .into_iter()
        .next()
        .context("latest L2 block not found")?;
    let l1_contract = profile
        .chain_spec
        .get_fork_l1_contract_address_at(
            latest_l2_block.header.number,
            latest_l2_block.header.timestamp,
        )
        .with_context(|| {
            format!(
                "resolve active Shasta L1 contract for {} at L2 block {} timestamp {}",
                profile.chain_spec.name,
                latest_l2_block.header.number,
                latest_l2_block.header.timestamp
            )
        })?;

    let latest_l1_block_number = l1_provider
        .get_block_number()
        .await
        .context("fetch latest L1 block number")?;
    let latest_proposal = fetch_latest_proposal(
        &l1_provider,
        l1_contract,
        latest_l1_block_number,
        args.l1_scan_page_size,
        args.l1_max_lookback,
    )
    .await?;
    let proposal_range = fetch_l2_proposal_range(
        &l2_client,
        latest_l2_block_number,
        latest_proposal.proposal_id,
        args.l2_scan_page_size,
        args.l2_max_lookback,
    )
    .await?;

    let request = BatchRequestFile {
        proposals: vec![ProposalRequestFile {
            proposal_id: latest_proposal.proposal_id,
            l1_inclusion_block_number: latest_proposal.l1_inclusion_block_number,
            l2_block_numbers: (proposal_range.start..=proposal_range.end).collect(),
            last_anchor_block_number: args.last_anchor_block_number,
        }],
        aggregate: args.aggregate,
        proof_type: args.proof_type.as_str(),
        network: profile.network,
        l1_network: profile.l1_network,
        blob_proof_type: args.blob_proof_type,
    };

    let json = serde_json::to_vec_pretty(&request).context("serialize latest proposal request")?;
    if let Some(output) = args.output.as_ref() {
        let path = resolve_output_path(root, output);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create output dir {}", parent.display()))?;
        }
        std::fs::write(&path, &json).with_context(|| format!("write {}", path.display()))?;
        eprintln!(
            "Wrote latest proposal request to {} (proposal_id={}, l2_blocks={}..={})",
            path.display(),
            latest_proposal.proposal_id,
            proposal_range.start,
            proposal_range.end
        );
    } else {
        println!(
            "{}",
            String::from_utf8(json).context("latest proposal request JSON was not valid UTF-8")?
        );
    }

    Ok(())
}

fn validate_args(args: &LatestProposalRequestArgs) -> Result<()> {
    if args.l1_scan_page_size == 0 {
        bail!("--l1-scan-page-size must be greater than zero");
    }
    if args.l2_scan_page_size == 0 {
        bail!("--l2-scan-page-size must be greater than zero");
    }
    if args.l1_max_lookback == 0 {
        bail!("--l1-max-lookback must be greater than zero");
    }
    if args.l2_max_lookback == 0 {
        bail!("--l2-max-lookback must be greater than zero");
    }
    Ok(())
}

fn load_supported_chain_specs(chain_spec_file: Option<&PathBuf>) -> Result<SupportedChainSpecs> {
    match chain_spec_file {
        Some(path) => SupportedChainSpecs::merge_from_file(path.clone()),
        None => Ok(SupportedChainSpecs::default()),
    }
}

fn resolve_profile(
    chain_specs: &SupportedChainSpecs,
    args: &LatestProposalRequestArgs,
) -> Result<ResolvedProfile> {
    let (network, l1_network, fallback_l1_rpc, fallback_l2_rpc) = match args.profile {
        LatestProposalProfile::TaikoMainnet => {
            ("taiko_mainnet", "ethereum", DEFAULT_ETHEREUM_RPC_URL, None)
        }
        LatestProposalProfile::TaikoHoodi => (
            "taiko_hoodi",
            "hoodi",
            DEFAULT_HOODI_RPC_URL,
            Some(DEFAULT_HOODI_TAIKO_RPC_URL),
        ),
    };

    let chain_spec = chain_specs
        .get_chain_spec(network)
        .with_context(|| format!("missing built-in chain spec for network {network}"))?;
    let l2_rpc_url = args.l2_rpc_url.clone().unwrap_or_else(|| {
        if chain_spec.rpc.is_empty() {
            fallback_l2_rpc.unwrap_or_default().to_string()
        } else {
            chain_spec.rpc.clone()
        }
    });
    if l2_rpc_url.is_empty() {
        bail!("no L2 RPC URL resolved for profile {network}; pass --l2-rpc-url");
    }

    Ok(ResolvedProfile {
        network,
        l1_network,
        l1_rpc_url: args
            .l1_rpc_url
            .clone()
            .unwrap_or_else(|| fallback_l1_rpc.to_string()),
        l2_rpc_url,
        chain_spec,
    })
}

fn resolve_output_path(root: &Path, requested: &Path) -> PathBuf {
    if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        root.join(requested)
    }
}

async fn fetch_latest_proposal<P>(
    provider: &P,
    l1_contract: alloy::primitives::Address,
    latest_block_number: u64,
    page_size: u64,
    max_lookback: u64,
) -> Result<LatestProposalRef>
where
    P: AlloyProvider,
{
    let earliest_block_number = latest_block_number.saturating_sub(max_lookback.saturating_sub(1));
    let mut upper = latest_block_number;

    loop {
        let lower = upper
            .saturating_sub(page_size.saturating_sub(1))
            .max(earliest_block_number)
            .max(1);
        let filter = Filter::new()
            .address(l1_contract)
            .from_block(lower)
            .to_block(upper)
            .event_signature(Proposed::SIGNATURE_HASH);
        let logs = provider
            .get_logs(&filter)
            .await
            .with_context(|| format!("fetch Proposed logs in L1 block range [{lower}, {upper}]"))?;
        if let Some(candidate) = select_latest_proposal_ref(&logs)? {
            return Ok(candidate);
        }
        if lower == earliest_block_number || lower == 1 {
            break;
        }
        upper = lower - 1;
    }

    bail!(
        "no Proposed event found in the latest {max_lookback} L1 blocks for contract {l1_contract:#x}"
    )
}

fn select_latest_proposal_ref(logs: &[RpcLog]) -> Result<Option<LatestProposalRef>> {
    let mut latest: Option<(u64, u64, LatestProposalRef)> = None;
    for log in logs {
        let Some(block_number) = log.block_number else {
            continue;
        };
        let log_index = log.log_index.unwrap_or_default();
        let event = decode_proposal_log(log)?;
        let candidate = LatestProposalRef {
            proposal_id: event.data.id.to::<u64>(),
            l1_inclusion_block_number: block_number,
        };
        match latest {
            Some((best_block, best_index, _))
                if best_block > block_number
                    || (best_block == block_number && best_index >= log_index) => {}
            _ => latest = Some((block_number, log_index, candidate)),
        }
    }
    Ok(latest.map(|(_, _, candidate)| candidate))
}

fn decode_proposal_log(log: &RpcLog) -> Result<alloy::primitives::Log<Proposed>> {
    let Some(log_struct) = PrimitiveLog::new(
        log.address(),
        log.topics().to_vec(),
        log.data().data.clone(),
    ) else {
        bail!("failed to decode Shasta proposal log envelope");
    };
    Proposed::decode_log(&log_struct).context("decode Shasta Proposed log")
}

async fn fetch_l2_proposal_range(
    client: &alloy::rpc::client::RpcClient,
    latest_block_number: u64,
    proposal_id: u64,
    page_size: u64,
    max_lookback: u64,
) -> Result<L2ProposalRange> {
    let earliest_block_number = latest_block_number.saturating_sub(max_lookback.saturating_sub(1));
    let mut upper = latest_block_number;
    let mut finder = DescendingRangeFinder::new(proposal_id);

    loop {
        let lower = upper
            .saturating_sub(page_size.saturating_sub(1))
            .max(earliest_block_number);
        let block_numbers = (lower..=upper).collect::<Vec<_>>();
        let blocks = fetch_blocks(client, &block_numbers).await?;
        for block in blocks.iter().rev() {
            if let Some(range) = finder.observe(block.header.number, block_proposal_id(block))? {
                return Ok(range);
            }
        }
        if lower == earliest_block_number {
            break;
        }
        upper = lower - 1;
    }

    if earliest_block_number == 0 && finder.start == Some(0) {
        return Ok(L2ProposalRange {
            start: 0,
            end: finder.end.context("proposal range end missing")?,
        });
    }

    if let Some(range) = finder.finish_closed() {
        return Ok(range);
    }

    if finder.seen_any() {
        bail!(
            "proposal_id {} extends beyond the latest {} L2 blocks; increase --l2-max-lookback",
            proposal_id,
            max_lookback
        );
    }

    bail!(
        "proposal_id {} not found in the latest {} L2 blocks",
        proposal_id,
        max_lookback
    )
}

async fn fetch_blocks(
    client: &alloy::rpc::client::RpcClient,
    block_numbers: &[u64],
) -> Result<Vec<RethBlock>> {
    let mut blocks = Vec::with_capacity(block_numbers.len());
    for chunk in block_numbers.chunks(BLOCK_BATCH_SIZE) {
        let mut batch = client.new_batch();
        let mut requests = Vec::with_capacity(chunk.len());
        for &block_number in chunk {
            requests.push((
                block_number,
                Box::pin(
                    batch
                        .add_call::<_, Option<AlloyBlock>>(
                            "eth_getBlockByNumber",
                            &(BlockNumberOrTag::from(block_number), true),
                        )
                        .map_err(|_| {
                            anyhow::anyhow!(
                                "failed adding eth_getBlockByNumber call to batch for block {block_number}"
                            )
                        })?,
                ),
            ));
        }

        batch
            .send()
            .await
            .with_context(|| format!("send eth_getBlockByNumber batch for blocks {chunk:?}"))?;

        for (block_number, request) in requests {
            let rpc_block: AlloyBlock = request
                .await
                .with_context(|| format!("collect eth_getBlockByNumber for block {block_number}"))?
                .with_context(|| format!("block {block_number} not found"))?;
            blocks.push(rpc_block.into());
        }
    }
    Ok(blocks)
}

fn block_proposal_id(block: &RethBlock) -> Option<u64> {
    decode_shasta_proposal_id(block.header.extra_data.as_ref())
}

fn decode_shasta_proposal_id(extra_data: &[u8]) -> Option<u64> {
    if extra_data.len() < 7 {
        return None;
    }
    let mut proposal_bytes = [0_u8; 8];
    proposal_bytes[2..8].copy_from_slice(&extra_data[1..7]);
    Some(u64::from_be_bytes(proposal_bytes))
}

impl RequestProofType {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::Sp1 => "sp1",
            Self::Risc0 => "risc0",
            Self::ZkAny => "zk_any",
        }
    }
}

impl DescendingRangeFinder {
    const fn new(target_proposal_id: u64) -> Self {
        Self {
            target_proposal_id,
            start: None,
            end: None,
            closed: false,
        }
    }

    fn observe(
        &mut self,
        block_number: u64,
        proposal_id: Option<u64>,
    ) -> Result<Option<L2ProposalRange>> {
        if self.end.is_none() {
            if proposal_id == Some(self.target_proposal_id) {
                self.start = Some(block_number);
                self.end = Some(block_number);
            }
            return Ok(None);
        }

        if proposal_id == Some(self.target_proposal_id) {
            let current_start = self.start.context("range start missing")?;
            if block_number + 1 != current_start {
                bail!(
                    "proposal_id {} L2 blocks are not contiguous around {} -> {}",
                    self.target_proposal_id,
                    block_number,
                    current_start
                );
            }
            self.start = Some(block_number);
            return Ok(None);
        }

        self.closed = true;
        Ok(self.finish_closed())
    }

    fn finish_closed(&self) -> Option<L2ProposalRange> {
        if !self.closed {
            return None;
        }
        let start = self.start?;
        let end = self.end?;
        Some(L2ProposalRange { start, end })
    }

    const fn seen_any(&self) -> bool {
        self.end.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DescendingRangeFinder, L2ProposalRange, LatestProposalRef, decode_shasta_proposal_id,
    };
    use alloy::primitives::Bytes;

    fn shasta_extra_data(proposal_id: u64) -> Bytes {
        let proposal_bytes = proposal_id.to_be_bytes();
        vec![
            0,
            proposal_bytes[2],
            proposal_bytes[3],
            proposal_bytes[4],
            proposal_bytes[5],
            proposal_bytes[6],
            proposal_bytes[7],
        ]
        .into()
    }

    #[test]
    fn decodes_shasta_proposal_id_from_extra_data() {
        let proposal_id = 2_670_u64;
        let decoded = decode_shasta_proposal_id(shasta_extra_data(proposal_id).as_ref());
        assert_eq!(decoded, Some(proposal_id));
    }

    #[test]
    fn returns_none_for_short_extra_data() {
        assert_eq!(decode_shasta_proposal_id(&[1, 2, 3]), None);
    }

    #[test]
    fn descending_range_finder_closes_on_previous_proposal() {
        let mut finder = DescendingRangeFinder::new(42);
        assert_eq!(finder.observe(106, Some(99)).expect("skip"), None);
        assert_eq!(finder.observe(105, Some(42)).expect("start"), None);
        assert_eq!(finder.observe(104, Some(42)).expect("extend"), None);
        assert_eq!(
            finder.observe(103, Some(41)).expect("close"),
            Some(L2ProposalRange {
                start: 104,
                end: 105,
            })
        );
    }

    #[test]
    fn descending_range_finder_rejects_non_contiguous_segments() {
        let mut finder = DescendingRangeFinder::new(42);
        finder.observe(105, Some(42)).expect("start");
        let err = finder.observe(103, Some(42)).expect_err("gap should fail");
        assert!(err.to_string().contains("not contiguous"));
    }

    #[test]
    fn latest_proposal_ref_is_copyable() {
        let proposal = LatestProposalRef {
            proposal_id: 7,
            l1_inclusion_block_number: 11,
        };
        let copied = proposal;
        assert_eq!(copied.proposal_id, 7);
        assert_eq!(copied.l1_inclusion_block_number, 11);
    }
}
