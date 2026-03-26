//! Shared fixture-backed local server harness.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use axum::{Json, Router, routing::post};
use raiko2_engine::{Engine, EngineObserver};
use raiko2_pipeline::{
    NativeBackend, PipelineKey, Risc0ShastaBackend, Sp1ShastaBackend,
    forks::shasta::{RISC0_SHASTA_BACKEND, SP1_SHASTA_BACKEND, ShastaSpec},
};
use raiko2_primitives::{ProofContext, ProofRequest, RaikoError, RaikoResult};
use raiko2_primitives_shasta::{GuestInput, build_proof_carry_data};
use raiko2_protocol_shasta::shasta::ProofCarryData;
use raiko2_prover::{
    native::NativeProver,
    risc0::{Risc0Config, Risc0Prover},
    sp1::{ExecutionMode, ProverMode, RecursionMode, Sp1Config, Sp1Prover},
};
use raiko2_provider::Provider;
use raiko2_queue::{MemoryStore, RetryPolicy, SchedulerConfig};
use raiko2_runtime::RuntimeManager;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tracing::info;

use super::AppState;
use super::app;
use super::net;
use super::state::{RuntimeObserver, StaticPipelineFactory};
use crate::cli::FixtureServerArgs;
use crate::config::{Config, GuestSystem, NetworkPairConfig, RunnerKind};

pub(crate) type NativeFixtureSpec = ShastaSpec<NativeProver, NativeBackend, FixtureProvider>;
pub(crate) type NativeFixtureEngine = Engine<NativeFixtureSpec>;
pub(crate) type Risc0FixtureSpec = ShastaSpec<Risc0Prover, Risc0ShastaBackend, FixtureProvider>;
pub(crate) type Risc0FixtureEngine = Engine<Risc0FixtureSpec>;
pub(crate) type Sp1FixtureSpec = ShastaSpec<Sp1Prover, Sp1ShastaBackend, FixtureProvider>;
pub(crate) type Sp1FixtureEngine = Engine<Sp1FixtureSpec>;

pub(crate) fn unique_runtime_root(prefix: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "{prefix}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos()
    ))
}

#[derive(Clone)]
pub(crate) struct FixtureProvider {
    input: Arc<GuestInput>,
}

impl FixtureProvider {
    #[must_use]
    pub(crate) fn from_repo_test_json() -> Self {
        let raw = include_str!("../../../../test.json");
        let mut input: GuestInput =
            serde_json::from_str(raw).expect("parse test.json as GuestInput");
        if input.proof_carry_data == ProofCarryData::default() && !input.witnesses.is_empty() {
            input.proof_carry_data = build_proof_carry_data(&input);
        }
        Self {
            input: Arc::new(input),
        }
    }

    fn witness_for_block(&self, block_number: u64) -> Option<&raiko2_primitives::StatelessInput> {
        self.input
            .witnesses
            .iter()
            .find(|w| w.block.header.number == block_number)
    }
}

#[async_trait::async_trait]
impl Provider for FixtureProvider {
    async fn batch_blocks(
        &self,
        block_numbers: &[u64],
    ) -> RaikoResult<Vec<reth_ethereum_primitives::Block>> {
        let mut out = Vec::with_capacity(block_numbers.len());
        for block_number in block_numbers {
            let witness = self
                .witness_for_block(*block_number)
                .ok_or_else(|| RaikoError::RPC(format!("fixture missing block {block_number}")))?;
            out.push(witness.block.clone());
        }
        Ok(out)
    }

    async fn batch_accounts(
        &self,
        block_numbers: &[u64],
        _accounts: &[Vec<alloy_primitives::Address>],
    ) -> RaikoResult<Vec<alloy_primitives::map::AddressMap<alloy_trie::TrieAccount>>> {
        let mut out = Vec::with_capacity(block_numbers.len());
        for block_number in block_numbers {
            let witness = self.witness_for_block(*block_number).ok_or_else(|| {
                RaikoError::RPC(format!("fixture missing accounts for block {block_number}"))
            })?;
            out.push(witness.accounts.clone());
        }
        Ok(out)
    }

    async fn batch_witnesses(
        &self,
        block_numbers: &[u64],
    ) -> RaikoResult<Vec<raiko2_primitives::ExecutionWitness>> {
        let mut out = Vec::with_capacity(block_numbers.len());
        for block_number in block_numbers {
            let witness = self.witness_for_block(*block_number).ok_or_else(|| {
                RaikoError::RPC(format!("fixture missing witness for block {block_number}"))
            })?;
            out.push(witness.witness.clone());
        }
        Ok(out)
    }
}

#[must_use]
pub(crate) fn base_config() -> Config {
    let mut config = Config::default();
    config.prover.guest_system = GuestSystem::Risc0;
    config.prover.runner = RunnerKind::Local;
    config.rpc.pairs = vec![NetworkPairConfig {
        network: "taiko_dev".to_string(),
        l1_network: "ethereum".to_string(),
        l1_rpc: Some("http://localhost:8545".to_string()),
        l2_rpc: Some("http://localhost:9545".to_string()),
    }];
    config
}

const fn memory_scheduler_config() -> SchedulerConfig {
    SchedulerConfig {
        lease_duration: Duration::from_secs(60),
        retry: RetryPolicy::None,
    }
}

fn engine_observer(runtime: Arc<RuntimeManager>) -> Arc<dyn EngineObserver> {
    Arc::new(RuntimeObserver::new(runtime))
}

fn build_engine_with_observer<S>(
    spec: S,
    ctx: ProofContext,
    observer: Option<Arc<dyn EngineObserver>>,
) -> Engine<S>
where
    S: raiko2_pipeline::PipelineSpec,
    S::Prover: raiko2_prover::Prover<S::Backend, GuestInput = S::GuestInput>,
    S::Backend: raiko2_pipeline::ProverBackend,
    S::Provider: Provider,
{
    Engine::with_store_scheduler_config_and_observer(
        spec,
        ctx,
        MemoryStore::new(),
        memory_scheduler_config(),
        observer,
    )
}

#[cfg(test)]
pub(crate) fn native_fixture_engine() -> NativeFixtureEngine {
    native_fixture_engine_with_observer(None)
}

fn native_fixture_engine_with_observer(
    observer: Option<Arc<dyn EngineObserver>>,
) -> NativeFixtureEngine {
    let provider = FixtureProvider::from_repo_test_json();
    let spec = ShastaSpec::new(
        PipelineKey::ShastaNative,
        NativeProver,
        NativeBackend,
        provider,
    );
    let ctx = ProofContext::new(
        ProofRequest {
            l1_chain_id: 1,
            l2_chain_id: 167_001,
            proposal_id: 0,
            l2_block_range: None,
            proof_type: "native".to_string(),
            blob_proof_type: None,
            prover: None,
            graffiti: None,
        },
        raiko2_primitives::ProverConfig::default(),
    );
    build_engine_with_observer(spec, ctx, observer)
}

#[cfg(test)]
pub(crate) fn risc0_fixture_engine(context_config: serde_json::Value) -> Risc0FixtureEngine {
    risc0_fixture_engine_with_observer(context_config, None)
}

fn risc0_fixture_engine_with_observer(
    context_config: serde_json::Value,
    observer: Option<Arc<dyn EngineObserver>>,
) -> Risc0FixtureEngine {
    let provider = FixtureProvider::from_repo_test_json();
    let spec = ShastaSpec::new(
        PipelineKey::ShastaRisc0,
        Risc0Prover::new(Risc0Config {
            bonsai: false,
            snark: false,
            mock: true,
            profile: false,
            execution_po2: 20,
            verify: true,
        }),
        RISC0_SHASTA_BACKEND,
        provider,
    );
    let ctx = ProofContext::new(
        ProofRequest {
            l1_chain_id: 1,
            l2_chain_id: 167_001,
            proposal_id: 0,
            l2_block_range: None,
            proof_type: "risc0".to_string(),
            blob_proof_type: None,
            prover: None,
            graffiti: None,
        },
        context_config,
    );
    build_engine_with_observer(spec, ctx, observer)
}

#[cfg(test)]
pub(crate) fn sp1_fixture_engine(context_config: serde_json::Value) -> Sp1FixtureEngine {
    sp1_fixture_engine_with_observer(context_config, None)
}

fn sp1_fixture_engine_with_observer(
    context_config: serde_json::Value,
    observer: Option<Arc<dyn EngineObserver>>,
) -> Sp1FixtureEngine {
    let provider = FixtureProvider::from_repo_test_json();
    let spec = ShastaSpec::new(
        PipelineKey::ShastaSp1,
        Sp1Prover::new(Sp1Config {
            recursion: RecursionMode::Plonk,
            prover: Some(ProverMode::Local),
            mode: ExecutionMode::Prove,
            verify: true,
        }),
        SP1_SHASTA_BACKEND,
        provider,
    );
    let ctx = ProofContext::new(
        ProofRequest {
            l1_chain_id: 1,
            l2_chain_id: 167_001,
            proposal_id: 0,
            l2_block_range: None,
            proof_type: "sp1".to_string(),
            blob_proof_type: None,
            prover: None,
            graffiti: None,
        },
        context_config,
    );
    build_engine_with_observer(spec, ctx, observer)
}

#[cfg(test)]
pub(crate) fn app_with_engine<S>(
    config: Config,
    network_pair: &str,
    pipeline_key: PipelineKey,
    engine: Engine<S>,
) -> AppState
where
    S: raiko2_pipeline::PipelineSpec + Send + Sync + 'static,
    S::Prover: raiko2_prover::Prover<S::Backend, GuestInput = S::GuestInput> + 'static,
    S::Backend: raiko2_pipeline::ProverBackend + 'static,
    S::Provider: raiko2_provider::Provider + 'static,
{
    let mut factory = StaticPipelineFactory::default();
    factory.insert(network_pair.to_string(), pipeline_key, Arc::new(engine));
    AppState {
        config: Arc::new(config),
        pipelines: Arc::new(factory),
        runtime: Arc::new(
            RuntimeManager::new(unique_runtime_root("raiko2-e2e-runtime"))
                .expect("runtime manager"),
        ),
    }
}

#[cfg(test)]
pub(crate) fn app_with_native_fixture_engine(
    config: Config,
    engine: NativeFixtureEngine,
) -> Router {
    let state = app_with_engine(
        config,
        "taiko_dev/ethereum",
        PipelineKey::ShastaNative,
        engine,
    );
    app::build_router(state)
}

pub(crate) async fn spawn_chain_id_rpc(
    chain_id: u64,
) -> Result<(String, tokio::task::JoinHandle<()>), std::io::Error> {
    let app = Router::new().route(
        "/",
        post(move |Json(req): Json<Value>| async move {
            let id = req.get("id").cloned().unwrap_or(Value::Null);
            let method = req
                .get("method")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if method == "eth_chainId" {
                Json(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": format!("0x{:x}", chain_id),
                }))
            } else {
                Json(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32601, "message": format!("method not found: {method}") },
                }))
            }
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr().expect("listener local_addr");
    let url = format!("http://{addr}");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve mock rpc");
    });
    Ok((url, handle))
}

fn fixture_app_state(config: Config) -> Result<AppState> {
    let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
        "raiko2-fixture-runtime",
    ))?);
    let observer = engine_observer(Arc::clone(&runtime));
    let maintenance_interval = Duration::from_millis(config.queue.maintenance_interval_ms);
    let workers = config.queue.workers;

    let mut factory = StaticPipelineFactory::default();

    let native_engine = native_fixture_engine_with_observer(Some(Arc::clone(&observer)));
    native_engine.start_workers_with_maintenance_interval(workers, maintenance_interval);
    factory.insert(
        "taiko_dev/ethereum".to_string(),
        PipelineKey::ShastaNative,
        Arc::new(native_engine),
    );

    let risc0_engine = risc0_fixture_engine_with_observer(json!({}), Some(Arc::clone(&observer)));
    risc0_engine.start_workers_with_maintenance_interval(workers, maintenance_interval);
    factory.insert(
        "taiko_dev/ethereum".to_string(),
        PipelineKey::ShastaRisc0,
        Arc::new(risc0_engine),
    );

    let sp1_engine = sp1_fixture_engine_with_observer(json!({}), Some(observer));
    sp1_engine.start_workers_with_maintenance_interval(workers, maintenance_interval);
    factory.insert(
        "taiko_dev/ethereum".to_string(),
        PipelineKey::ShastaSp1,
        Arc::new(sp1_engine),
    );

    Ok(AppState {
        config: Arc::new(config),
        pipelines: Arc::new(factory),
        runtime,
    })
}

pub async fn run_fixture_server(args: &FixtureServerArgs) -> Result<()> {
    let (l2_rpc, chain_id_handle) = spawn_chain_id_rpc(167_001).await?;

    let mut config = base_config();
    config.server.host = args.host.clone();
    config.server.port = args.port;
    config.rpc.pairs[0].l2_rpc = Some(l2_rpc);
    config.queue.workers = args.workers;

    let state = fixture_app_state(config.clone())?;
    let app = app::build_router(state);
    let addr = net::bind_addr(&config);
    let listener = TcpListener::bind(&addr).await?;

    info!(
        "Fixture server listening on http://{} for network pair taiko_dev/ethereum",
        addr
    );
    info!(
        "Try POST http://{}/v3/proof/batch/shasta with proof_type=sp1 and sp1.mode=execute",
        addr
    );

    let result = axum::serve(listener, app).await;
    chain_id_handle.abort();
    result?;
    Ok(())
}
