//! Dumps the exact preflight account and storage-key access set for each block in a proposal.

use alethia_reth_block::config::TaikoEvmConfig;
use alethia_reth_chainspec::TAIKO_HOODI;
use alloy::consensus::Transaction;
use alloy::providers::ProviderBuilder;
use alloy_primitives::{Address, B256};
use anyhow::{Context, Result, bail};
use clap::Parser;
use raiko2_primitives_shasta::GuestInput;
use raiko2_provider::on_the_spot_witness::{PreflightDb, ProviderConfig, ProviderDb};
use raiko2_provider::rpc::{RpcClientConfig, build_rpc_client};
use reth_evm::{ConfigureEvm, execute::Executor};
use reth_primitives_traits::{Block as _, RecoveredBlock};
use serde::Serialize;
use std::{fs, path::PathBuf, sync::Arc};
use tracing::Span;

#[derive(Debug, Parser)]
struct Args {
    #[arg(long)]
    input: PathBuf,
    #[arg(long)]
    rpc_url: String,
    #[arg(long)]
    output: PathBuf,
    #[arg(long, default_value_t = 600_000)]
    rpc_timeout_ms: u64,
}

#[derive(Debug, Serialize)]
struct AccessDump {
    proposal_id: u64,
    chain_id: u64,
    blocks: Vec<BlockAccessDump>,
}

#[derive(Debug, Serialize)]
struct BlockAccessDump {
    block_number: u64,
    addresses: Vec<AddressAccessDump>,
}

#[derive(Debug, Serialize)]
struct AddressAccessDump {
    address: Address,
    slots: Vec<B256>,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let args = Args::parse();
    let input = fs::read_to_string(&args.input)
        .with_context(|| format!("read {}", args.input.display()))?;
    let proposal: GuestInput =
        serde_json::from_str(&input).context("parse proposal input with current schema")?;
    let proposal_id = proposal.taiko.proposal_id;
    let chain_id = proposal.taiko.chain_spec.chain_id;

    let rpc_client = build_rpc_client(
        &args.rpc_url,
        &RpcClientConfig {
            timeout_ms: args.rpc_timeout_ms,
            ..RpcClientConfig::default()
        },
    )?;
    let provider = ProviderBuilder::new().connect_client(rpc_client);
    let evm_config = match chain_id {
        167013 => Arc::new(TaikoEvmConfig::new(TAIKO_HOODI.clone())),
        _ => bail!("unsupported chain_id {chain_id}"),
    };

    let mut dumps = Vec::with_capacity(proposal.witnesses.len());
    for witness in proposal.witnesses {
        let block = witness.block;
        let block_number = block.header.number;
        let parent_hash = block.header.parent_hash;
        let recovered_block: RecoveredBlock<_> = block
            .clone()
            .try_into_recovered()
            .context("recover block senders")?;

        let mut db = PreflightDb::new(ProviderDb::new(
            provider.clone(),
            ProviderConfig::default(),
            parent_hash,
        ));

        for tx in recovered_block.body().transactions() {
            if let Some(access_list) = tx.access_list() {
                db.add_access_list(access_list).await?;
            }
        }

        let current_span = Span::current();
        let evm_config = Arc::clone(&evm_config);
        let (execution_result, captured_db) = tokio::task::spawn_blocking(move || {
            current_span.in_scope(|| {
                let executor = evm_config.executor(db);
                let mut database_capture = None;
                let outcome = executor.execute_with_state_closure(&recovered_block, |state| {
                    database_capture = Some(Box::new(state.database.clone()));
                });
                (outcome, database_capture)
            })
        })
        .await
        .context("join block execution task")?;
        execution_result.context("execute block for preflight access capture")?;
        let db = *captured_db.context("preflight db capture missing after execution")?;

        let mut addresses = db
            .accessed_storage_keys()
            .into_iter()
            .map(|(address, mut slots)| {
                slots.sort_unstable();
                AddressAccessDump { address, slots }
            })
            .collect::<Vec<_>>();
        addresses.sort_by_key(|entry| entry.address);

        dumps.push(BlockAccessDump {
            block_number,
            addresses,
        });
    }

    let dump = AccessDump {
        proposal_id,
        chain_id,
        blocks: dumps,
    };

    let output = serde_json::to_vec_pretty(&dump).context("serialize access dump")?;
    fs::write(&args.output, output).with_context(|| format!("write {}", args.output.display()))?;
    Ok(())
}
