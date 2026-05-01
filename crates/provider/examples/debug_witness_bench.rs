//! Benchmark `debug_executionWitness` via the repo RPC client configuration.

use std::time::Instant;

use alloy::{
    eips::BlockNumberOrTag,
    providers::{Provider as AlloyProvider, ProviderBuilder},
};
use alloy_rpc_types_eth::Block as AlloyBlock;
use clap::Parser;
use futures::{StreamExt, stream};
use raiko2_primitives::ExecutionWitness;
use raiko2_provider::rpc::{RpcClientConfig, RpcRetryConfig, build_rpc_client};

#[derive(Parser, Debug)]
struct Args {
    #[arg(long)]
    rpc_url: String,

    #[arg(long, value_delimiter = ',', default_values_t = [6183120_u64, 6183121_u64])]
    blocks: Vec<u64>,

    #[arg(long, default_value = "1x2,2x4,3x6")]
    scenarios: String,

    #[arg(long, default_value_t = 600_000)]
    rpc_timeout_ms: u64,

    #[arg(long, default_value_t = 32)]
    rpc_concurrency_limit: usize,

    #[arg(long, default_value_t = 4)]
    rpc_retry_max_attempts: u32,

    #[arg(long, default_value_t = 1_000)]
    rpc_retry_initial_backoff_ms: u64,

    #[arg(long, default_value_t = 1_000)]
    rpc_retry_cu_per_second: u64,

    #[arg(long, default_value_t = false)]
    warmup_blocks: bool,
}

#[derive(Clone, Copy, Debug)]
struct Scenario {
    concurrency: usize,
    total_requests: usize,
}

#[derive(Debug)]
struct CallResult {
    ok: bool,
    elapsed_ms: u128,
    state_nodes: usize,
    error: Option<String>,
}

fn parse_scenarios(raw: &str) -> Result<Vec<Scenario>, String> {
    raw.split(',')
        .filter(|part| !part.trim().is_empty())
        .map(|part| {
            let (concurrency, total_requests) = part.split_once('x').ok_or_else(|| {
                format!("invalid scenario '{part}', expected <concurrency>x<count>")
            })?;
            let concurrency = concurrency
                .parse::<usize>()
                .map_err(|err| format!("parse concurrency from '{part}': {err}"))?;
            let total_requests = total_requests
                .parse::<usize>()
                .map_err(|err| format!("parse request count from '{part}': {err}"))?;
            if concurrency == 0 || total_requests == 0 {
                return Err(format!("scenario '{part}' must be > 0"));
            }
            Ok(Scenario {
                concurrency,
                total_requests,
            })
        })
        .collect()
}

fn summarize_latency(values: &[u128]) -> Option<(u128, u128, u128, u128)> {
    if values.is_empty() {
        return None;
    }

    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let min = *sorted.first().unwrap_or(&0);
    let max = *sorted.last().unwrap_or(&0);
    let p50 = sorted[sorted.len() / 2];
    let avg = sorted.iter().sum::<u128>() / u128::try_from(sorted.len()).unwrap_or(1);
    Some((min, p50, max, avg))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let scenarios = parse_scenarios(&args.scenarios)?;
    let client = build_rpc_client(
        &args.rpc_url,
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
    let provider = ProviderBuilder::new().connect_client(client);

    if args.warmup_blocks {
        for &block_number in &args.blocks {
            let response: Option<AlloyBlock> = provider
                .client()
                .request(
                    "eth_getBlockByNumber",
                    (BlockNumberOrTag::from(block_number), true),
                )
                .await?;
            println!("warmup block={} found={}", block_number, response.is_some());
        }
    }

    for scenario in scenarios {
        let started = Instant::now();
        let results = stream::iter(0..scenario.total_requests)
            .map(|index| {
                let provider = provider.clone();
                let block_number = args.blocks[index % args.blocks.len()];
                async move {
                    let call_started = Instant::now();
                    let response: Result<ExecutionWitness, _> = provider
                        .client()
                        .request(
                            "debug_executionWitness",
                            (BlockNumberOrTag::from(block_number),),
                        )
                        .await;
                    match response {
                        Ok(witness) => CallResult {
                            ok: true,
                            elapsed_ms: call_started.elapsed().as_millis(),
                            state_nodes: witness.state.len(),
                            error: None,
                        },
                        Err(err) => CallResult {
                            ok: false,
                            elapsed_ms: call_started.elapsed().as_millis(),
                            state_nodes: 0,
                            error: Some(err.to_string()),
                        },
                    }
                }
            })
            .buffer_unordered(scenario.concurrency)
            .collect::<Vec<_>>()
            .await;

        let successes = results
            .iter()
            .filter(|result| result.ok)
            .collect::<Vec<_>>();
        let failures = results
            .iter()
            .filter(|result| !result.ok)
            .collect::<Vec<_>>();
        let success_latencies = successes
            .iter()
            .map(|result| result.elapsed_ms)
            .collect::<Vec<_>>();
        let failure_latencies = failures
            .iter()
            .map(|result| result.elapsed_ms)
            .collect::<Vec<_>>();

        println!(
            "scenario concurrency={} requests={} wall_ms={} successes={} failures={}",
            scenario.concurrency,
            scenario.total_requests,
            started.elapsed().as_millis(),
            successes.len(),
            failures.len()
        );

        if let Some((min, p50, max, avg)) = summarize_latency(&success_latencies) {
            println!(
                "  success_latency_ms min={} p50={} max={} avg={}",
                min, p50, max, avg
            );
            println!(
                "  success_state_nodes sample={}",
                successes
                    .first()
                    .map(|result| result.state_nodes)
                    .unwrap_or(0)
            );
        }

        if let Some((min, p50, max, avg)) = summarize_latency(&failure_latencies) {
            println!(
                "  failure_latency_ms min={} p50={} max={} avg={}",
                min, p50, max, avg
            );
            if let Some(error) = failures.first().and_then(|result| result.error.as_deref()) {
                println!("  first_error={error}");
            }
        }
    }

    Ok(())
}
