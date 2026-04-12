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

use alloy::{
    consensus::transaction::SignerRecoverable,
    providers::{Provider as AlloyProvider, ProviderBuilder},
    rpc::client::RpcClient,
};
use anyhow::{Context, Result, bail};
use clap::Parser;
use raiko2_primitives::chain_spec::SupportedChainSpecs;
use raiko2_provider::{NetworkProvider, Provider};
use raiko2_stateless::validate_block;
use reth_ethereum_primitives::Block;
use std::time::{Duration, Instant};

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
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
}

#[tokio::main]
async fn main() -> Result<()> {
    // Enable tracing subscriber for debug/pretty log output.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("info".parse().unwrap()),
        )
        .init();

    let args = Args::parse();

    let mut metrics = RunMetrics::new(args.metrics);

    let url = reqwest::Url::parse(&args.rpc_url).context("Invalid rpc_url")?;
    let rpc_client = RpcClient::builder().http(url);
    let rpc_provider = ProviderBuilder::new().connect_client(rpc_client);

    let chain_id = if let Some(chain_id) = args.chain_id {
        chain_id
    } else {
        let start = Instant::now();
        let res = rpc_provider.get_chain_id().await;
        metrics.observe("rpc.eth_chainId", start.elapsed(), 1);
        res.context("eth_chainId failed")?
    };

    let chain_spec = SupportedChainSpecs::default()
        .get_chain_spec_with_chain_id(chain_id)
        .context("Unsupported chain_id")?;
    if !chain_spec.is_taiko() {
        bail!("chain_id {chain_id} is not a Taiko chain");
    }

    let taiko_chain_spec = chain_spec
        .to_taiko_chain_spec()
        .context("Failed to convert to Taiko chain spec")?;
    let evm_config = alethia_reth_block::config::TaikoEvmConfig::new(taiko_chain_spec.clone());

    let provider = NetworkProvider::new(&args.rpc_url)?;

    let block_numbers = vec![args.block_number];
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

    metrics.set_block_stats(&blocks);

    for ((block, witness), callers) in blocks.into_iter().zip(witnesses).zip(accounts) {
        let start = Instant::now();
        let block_hash = validate_block(block, &witness, callers, &taiko_chain_spec, &evm_config)
            .context("Stateless validation failed")?;
        metrics.observe("stateless.validate_block", start.elapsed(), 1);
        println!("stateless validation ok: {block_hash:?}");
    }

    metrics.print_summary(chain_id, args.block_number);
    Ok(())
}

fn collect_signers(block: &Block) -> Vec<alloy_primitives::Address> {
    block
        .body
        .transactions()
        .filter_map(|tx| tx.recover_signer().ok())
        .collect()
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
