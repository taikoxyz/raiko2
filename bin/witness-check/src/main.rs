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

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Upstream RPC URL to fetch block/witness data.
    #[arg(long, env)]
    rpc_url: String,

    /// L2 block number to validate.
    #[arg(long)]
    block_number: u64,

    /// Explicit chain ID (defaults to eth_chainId if omitted).
    #[arg(long)]
    chain_id: Option<u64>,

    /// Whether the RPC supports debug_executionWitness.
    #[arg(long, value_parser = clap::builder::BoolishValueParser::new(), default_value = "false")]
    debug_witness: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let url = reqwest::Url::parse(&args.rpc_url).context("Invalid rpc_url")?;
    let rpc_client = RpcClient::builder().http(url);
    let rpc_provider = ProviderBuilder::new().connect_client(rpc_client);

    let chain_id = match args.chain_id {
        Some(chain_id) => chain_id,
        None => rpc_provider
            .get_chain_id()
            .await
            .context("eth_chainId failed")?,
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

    let provider =
        NetworkProvider::new(&args.rpc_url)?.with_debug_witness_support(args.debug_witness);

    let block_numbers = vec![args.block_number];
    let blocks = provider
        .batch_blocks(&block_numbers)
        .await
        .context("Failed to fetch blocks")?;
    let witnesses = provider
        .batch_witnesses(&block_numbers)
        .await
        .context("Failed to fetch witnesses")?;

    let signers = blocks.iter().map(collect_signers).collect::<Vec<_>>();
    let accounts = provider
        .batch_accounts(&block_numbers, &signers)
        .await
        .context("Failed to fetch accounts")?;

    if blocks.len() != witnesses.len() || blocks.len() != accounts.len() {
        bail!("Provider returned mismatched input lengths");
    }

    for ((block, witness), callers) in blocks.into_iter().zip(witnesses).zip(accounts) {
        let block_hash = validate_block(
            block,
            witness,
            callers,
            taiko_chain_spec.clone(),
            evm_config.clone(),
        )
        .context("Stateless validation failed")?;
        println!("stateless validation ok: {block_hash:?}");
    }

    Ok(())
}

fn collect_signers(block: &Block) -> Vec<alloy_primitives::Address> {
    block
        .body
        .transactions()
        .filter_map(|tx| tx.recover_signer().ok())
        .collect()
}
