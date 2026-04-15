//! CLI tool to validate stateless witnesses against a live RPC endpoint.
// Copyright 2025 RISC Zero, Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use alethia_reth_chainspec::spec::TaikoChainSpec;
use alloy::{
    consensus::TrieAccount,
    consensus::transaction::SignerRecoverable,
    primitives::address,
    providers::{Provider as AlloyProvider, ProviderBuilder},
    rpc::client::RpcClient,
    rpc::types::EIP1186AccountProofResponse,
};
use anyhow::{Context, Result, bail};
use clap::Parser;
use raiko2_primitives::chain_spec::SupportedChainSpecs;
use raiko2_primitives::{ExecutionWitness, WitnessStateNode};
use raiko2_provider::{NetworkProvider, Provider};
use raiko2_stateless::validate_block;
use reth_ethereum_primitives::Block;
use std::{
    collections::HashSet,
    panic::{AssertUnwindSafe, catch_unwind},
    path::PathBuf,
    time::{Duration, Instant},
};

const GOLDEN_TOUCH_ADDRESS: alloy_primitives::Address =
    address!("0000777735367b36bc9b61c50022d9d0700db4ec");

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Optional chain spec JSON file merged over the built-in chain spec list.
    #[arg(long)]
    chain_spec_file: Option<PathBuf>,

    /// Upstream RPC URL to fetch block/witness data.
    #[arg(long, env)]
    rpc_url: String,

    /// L2 block number to validate.
    #[arg(long)]
    block_number: u64,

    /// Explicit chain ID (defaults to `eth_chainId` if omitted).
    #[arg(long)]
    chain_id: Option<u64>,

    /// Print a small timing summary (network fetches + validation).
    #[arg(long, value_parser = clap::builder::BoolishValueParser::new(), default_value = "false")]
    metrics: bool,

    /// Compare the witness against the golden-touch account proof from `eth_getProof`.
    #[arg(long, value_parser = clap::builder::BoolishValueParser::new(), default_value = "false")]
    diagnose_golden_touch: bool,

    /// Temporarily merge the golden-touch account proof into the witness and validate again.
    #[arg(long, value_parser = clap::builder::BoolishValueParser::new(), default_value = "false")]
    supplement_golden_touch_proof: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let args = Args::parse();
    let mut metrics = RunMetrics::new(args.metrics);
    let rpc_provider = build_rpc_provider(&args)?;
    let chain_id = resolve_chain_id(&args, &rpc_provider, &mut metrics).await?;
    let supported_chain_specs = load_supported_chain_specs(args.chain_spec_file.as_ref())?;
    let chain_spec = supported_chain_specs
        .get_chain_spec_with_chain_id(chain_id)
        .context("Unsupported chain_id")?;
    if !chain_spec.is_taiko() {
        bail!("chain_id {chain_id} is not a Taiko chain");
    }

    let taiko_chain_spec = chain_spec
        .to_taiko_chain_spec()
        .context("Failed to convert to Taiko chain spec")?;
    let evm_config = alethia_reth_block::config::TaikoEvmConfig::new(taiko_chain_spec.clone());

    let provider = NetworkProvider::new_pair_with_chain_specs_and_config(
        &args.rpc_url,
        &args.rpc_url,
        None,
        Some(chain_spec.clone()),
        None,
        &raiko2_provider::RpcClientConfig::default(),
    )?;
    let mut validation_env = ValidationEnv {
        chain_spec: &taiko_chain_spec,
        evm_config: &evm_config,
        metrics: &mut metrics,
    };
    let (blocks, witnesses, accounts) =
        fetch_inputs(&provider, args.block_number, validation_env.metrics).await?;
    validation_env.metrics.set_block_stats(&blocks);

    for ((block, witness), callers) in blocks.into_iter().zip(witnesses).zip(accounts) {
        if maybe_validate_with_golden_touch(
            &args,
            &rpc_provider,
            &block,
            &witness,
            &callers,
            &mut validation_env,
        )
        .await?
        {
            continue;
        }

        let start = Instant::now();
        let block_hash =
            validate_block_captured(block, &witness, callers, &taiko_chain_spec, &evm_config)
                .context("Stateless validation failed")?;
        validation_env
            .metrics
            .observe("stateless.validate_block", start.elapsed(), 1);
        println!("stateless validation ok: {block_hash:?}");
    }

    validation_env
        .metrics
        .print_summary(chain_id, args.block_number);
    Ok(())
}

fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("info".parse().unwrap()),
        )
        .init();
}

fn build_rpc_provider(args: &Args) -> Result<impl AlloyProvider> {
    let url = reqwest::Url::parse(&args.rpc_url).context("Invalid rpc_url")?;
    let rpc_client = RpcClient::builder().http(url);
    Ok(ProviderBuilder::new().connect_client(rpc_client))
}

async fn resolve_chain_id<P: AlloyProvider>(
    args: &Args,
    rpc_provider: &P,
    metrics: &mut RunMetrics,
) -> Result<u64> {
    if let Some(chain_id) = args.chain_id {
        return Ok(chain_id);
    }

    let start = Instant::now();
    let res = rpc_provider.get_chain_id().await;
    metrics.observe("rpc.eth_chainId", start.elapsed(), 1);
    res.context("eth_chainId failed")
}

async fn fetch_inputs(
    provider: &NetworkProvider,
    block_number: u64,
    metrics: &mut RunMetrics,
) -> Result<(
    Vec<Block>,
    Vec<ExecutionWitness>,
    Vec<alloy_primitives::map::AddressMap<TrieAccount>>,
)> {
    let block_numbers = vec![block_number];
    let blocks = {
        let start = Instant::now();
        let res = provider.batch_blocks(&block_numbers).await;
        metrics.observe(
            "provider.batch_blocks",
            start.elapsed(),
            block_numbers.len(),
        );
        res.context("Failed to fetch blocks")?
    };
    let witnesses = {
        let start = Instant::now();
        let res = provider.batch_witnesses(&block_numbers).await;
        metrics.observe(
            "provider.batch_witnesses",
            start.elapsed(),
            block_numbers.len(),
        );
        res.context("Failed to fetch witnesses")?
    };
    let signers = blocks.iter().map(collect_signers).collect::<Vec<_>>();
    let signer_count = signers.iter().map(Vec::len).sum::<usize>();
    let accounts = {
        let start = Instant::now();
        let res = provider.batch_accounts(&block_numbers, &signers).await;
        metrics.observe("provider.batch_accounts", start.elapsed(), signer_count);
        res.context("Failed to fetch accounts")?
    };

    if blocks.len() != witnesses.len() || blocks.len() != accounts.len() {
        bail!("Provider returned mismatched input lengths");
    }

    Ok((blocks, witnesses, accounts))
}

struct ValidationEnv<'a> {
    chain_spec: &'a std::sync::Arc<TaikoChainSpec>,
    evm_config: &'a alethia_reth_block::config::TaikoEvmConfig,
    metrics: &'a mut RunMetrics,
}

async fn maybe_validate_with_golden_touch<P: AlloyProvider>(
    args: &Args,
    rpc_provider: &P,
    block: &Block,
    witness: &ExecutionWitness,
    callers: &alloy_primitives::map::AddressMap<TrieAccount>,
    validation_env: &mut ValidationEnv<'_>,
) -> Result<bool> {
    if !args.diagnose_golden_touch && !args.supplement_golden_touch_proof {
        return Ok(false);
    }

    let parent_block_number = block
        .header
        .number
        .checked_sub(1)
        .context("cannot diagnose golden-touch proof for genesis block")?;
    let proof = fetch_account_proof(rpc_provider, GOLDEN_TOUCH_ADDRESS, parent_block_number)
        .await
        .context("fetch golden-touch account proof")?;
    print_golden_touch_coverage(witness, &proof);

    if !args.supplement_golden_touch_proof {
        return Ok(false);
    }

    let mut supplemented_witness = witness.clone();
    supplemented_witness.state.extend(
        proof
            .account_proof
            .iter()
            .cloned()
            .map(WitnessStateNode::from_bytes),
    );
    supplemented_witness.state =
        ExecutionWitness::canonicalize_state_nodes(supplemented_witness.state);

    let start = Instant::now();
    let block_hash = validate_block_captured(
        block.clone(),
        &supplemented_witness,
        callers.clone(),
        validation_env.chain_spec,
        validation_env.evm_config,
    )
    .context("Stateless validation failed after supplementing golden-touch proof")?;
    validation_env
        .metrics
        .observe("stateless.validate_block.supplemented", start.elapsed(), 1);
    println!("stateless validation ok after supplement: {block_hash:?}");
    Ok(true)
}

async fn fetch_account_proof<P: AlloyProvider>(
    rpc_provider: &P,
    address: alloy_primitives::Address,
    block_number: u64,
) -> Result<EIP1186AccountProofResponse> {
    rpc_provider
        .get_proof(address, Vec::new())
        .number(block_number)
        .await
        .context("eth_getProof failed")
}

fn print_golden_touch_coverage(witness: &ExecutionWitness, proof: &EIP1186AccountProofResponse) {
    let witness_hashes = witness
        .state
        .iter()
        .map(|node| node.hash)
        .collect::<HashSet<_>>();
    let missing = proof
        .account_proof
        .iter()
        .map(alloy_primitives::keccak256)
        .filter(|hash| !witness_hashes.contains(hash))
        .collect::<Vec<_>>();

    eprintln!(
        "golden_touch_proof_nodes={} missing_from_witness={} address={:?}",
        proof.account_proof.len(),
        missing.len(),
        proof.address
    );
    if !missing.is_empty() {
        eprintln!("golden_touch_missing_hashes={missing:?}");
    }
}

fn validate_block_captured(
    block: Block,
    witness: &ExecutionWitness,
    callers: alloy_primitives::map::AddressMap<TrieAccount>,
    chain_spec: &std::sync::Arc<TaikoChainSpec>,
    evm_config: &alethia_reth_block::config::TaikoEvmConfig,
) -> Result<alloy_primitives::B256> {
    match catch_unwind(AssertUnwindSafe(|| {
        validate_block(block, witness, callers, chain_spec, evm_config)
    })) {
        Ok(result) => result.context("validate_block returned error"),
        Err(payload) => bail!("validate_block panicked: {}", panic_message(payload)),
    }
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    match payload.downcast::<String>() {
        Ok(message) => *message,
        Err(payload) => match payload.downcast::<&'static str>() {
            Ok(message) => (*message).to_string(),
            Err(_) => "unknown panic payload".to_string(),
        },
    }
}

fn collect_signers(block: &Block) -> Vec<alloy_primitives::Address> {
    block
        .body
        .transactions()
        .filter_map(|tx| tx.recover_signer().ok())
        .collect()
}

fn load_supported_chain_specs(chain_spec_file: Option<&PathBuf>) -> Result<SupportedChainSpecs> {
    match chain_spec_file {
        Some(path) => SupportedChainSpecs::merge_from_file(path.clone()),
        None => Ok(SupportedChainSpecs::default()),
    }
}

#[derive(Debug, Default)]
struct RunMetrics {
    enabled: bool,
    observations: Vec<Observation>,
    block_count: usize,
    tx_count: usize,
}

#[derive(Debug)]
struct Observation {
    name: &'static str,
    duration: Duration,
    unit_count: usize,
}

impl RunMetrics {
    const fn new(enabled: bool) -> Self {
        Self {
            enabled,
            observations: Vec::new(),
            block_count: 0,
            tx_count: 0,
        }
    }

    fn observe(&mut self, name: &'static str, duration: Duration, unit_count: usize) {
        if !self.enabled {
            return;
        }
        self.observations.push(Observation {
            name,
            duration,
            unit_count,
        });
    }

    fn set_block_stats(&mut self, blocks: &[Block]) {
        if !self.enabled {
            return;
        }
        self.block_count = blocks.len();
        self.tx_count = blocks.iter().map(|b| b.body.transactions().count()).sum();
    }

    fn print_summary(&self, chain_id: u64, block_number: u64) {
        if !self.enabled {
            return;
        }

        let total = self
            .observations
            .iter()
            .fold(Duration::ZERO, |acc, o| acc.saturating_add(o.duration));

        eprintln!();
        eprintln!("=== witness-check metrics ===");
        eprintln!("chain_id: {chain_id}");
        eprintln!("block_number: {block_number}");
        eprintln!("blocks: {}", self.block_count);
        eprintln!("transactions: {}", self.tx_count);
        eprintln!("total_observed_time: {:.3}s", total.as_secs_f64());
        eprintln!();

        for o in &self.observations {
            let secs = o.duration.as_secs_f64();
            let unit_count = u32::try_from(o.unit_count).unwrap_or(u32::MAX);
            let rate = if secs > 0.0 {
                f64::from(unit_count) / secs
            } else {
                0.0
            };
            eprintln!(
                "{:<28} {:>9.3}s  units={:<6}  rate={:.1}/s",
                o.name, secs, o.unit_count, rate
            );
        }
    }
}
