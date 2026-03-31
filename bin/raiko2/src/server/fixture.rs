//! Shared fixture-backed local server harness.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use alloy::consensus::Header;
use alloy_primitives::{B256, Bytes};
use anyhow::Result;
use axum::{Json, Router, routing::post};
use raiko2_engine::{Engine, EngineObserver};
use raiko2_pipeline::{
    NativeBackend, NoopManifestBuilder, NoopValidation, PipelineKey, PipelineSpec, Preflight,
    ProverBackend, Risc0ShastaBackend, Sp1ShastaBackend,
    forks::shasta::{RISC0_SHASTA_BACKEND, SP1_SHASTA_BACKEND},
};
use raiko2_primitives::{Proof, ProofContext, ProofRequest, ProverConfig, RaikoError, RaikoResult};
use raiko2_primitives_shasta::{GuestInput, encode_proof_carry_data};
use raiko2_protocol_shasta::shasta::ShastaEventData;
use raiko2_prover::{
    GuestInputCodec, Prover,
    native::NativeProver,
    sp1::{ExecutionMode, ProverMode, RecursionMode, Sp1Config, Sp1ConfigOverrides},
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

pub(crate) type NativeFixtureSpec = FixtureSpec<NativeProver, NativeBackend>;
pub(crate) type NativeFixtureEngine = Engine<NativeFixtureSpec>;
pub(crate) type Risc0FixtureSpec = FixtureSpec<FixtureRisc0Prover, Risc0ShastaBackend>;
pub(crate) type Risc0FixtureEngine = Engine<Risc0FixtureSpec>;
pub(crate) type Sp1FixtureSpec = FixtureSpec<FixtureSp1Prover, Sp1ShastaBackend>;
pub(crate) type Sp1FixtureEngine = Engine<Sp1FixtureSpec>;

const FIXTURE_VALIDATION: NoopValidation<GuestInput> = NoopValidation::new();
const FIXTURE_MANIFEST: NoopManifestBuilder = NoopManifestBuilder;

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
        if input.taiko.l1_ancestor_headers.is_empty() && input.taiko.l1_header.number != 0 {
            input.taiko.l1_ancestor_headers = vec![input.taiko.l1_header.clone()];
        }
        Self {
            input: Arc::new(input),
        }
    }

    fn cloned_input(&self) -> GuestInput {
        (*self.input).clone()
    }

    fn witness_for_block(&self, block_number: u64) -> Option<&raiko2_primitives::StatelessInput> {
        self.input
            .witnesses
            .iter()
            .find(|w| w.block.header.number == block_number)
    }
}

#[derive(Clone)]
pub(crate) struct FixtureSpec<Pr, Bk> {
    pipeline_key: PipelineKey,
    prover: Pr,
    backend: Bk,
    provider: FixtureProvider,
}

impl<Pr, Bk> FixtureSpec<Pr, Bk> {
    const fn new(
        pipeline_key: PipelineKey,
        prover: Pr,
        backend: Bk,
        provider: FixtureProvider,
    ) -> Self {
        Self {
            pipeline_key,
            prover,
            backend,
            provider,
        }
    }
}

#[async_trait::async_trait]
impl<Pr, Bk> Preflight for FixtureSpec<Pr, Bk>
where
    Pr: Send + Sync,
    Bk: ProverBackend,
{
    type Input = GuestInput;

    async fn preflight<P: Provider>(
        &self,
        ctx: &ProofContext,
        _provider: &P,
    ) -> RaikoResult<Self::Input> {
        let mut input = self.provider.cloned_input();
        if let Some(data_sources) = ctx.config.get("shasta_data_sources") {
            input.taiko.data_sources =
                serde_json::from_value(data_sources.clone()).map_err(|err| {
                    RaikoError::InvalidRequestConfig(format!(
                        "invalid fixture shasta_data_sources override: {err}"
                    ))
                })?;
        }
        Ok(input)
    }
}

impl<Pr, Bk> PipelineSpec for FixtureSpec<Pr, Bk>
where
    Pr: Send + Sync,
    Bk: ProverBackend,
{
    type GuestInput = GuestInput;
    type Preflight = Self;
    type Validation = NoopValidation<GuestInput>;
    type ManifestBuilder = NoopManifestBuilder;
    type Prover = Pr;
    type Backend = Bk;
    type Provider = FixtureProvider;

    fn pipeline_key(&self) -> PipelineKey {
        self.pipeline_key
    }

    fn prover(&self) -> &Self::Prover {
        &self.prover
    }

    fn backend(&self) -> &Self::Backend {
        &self.backend
    }

    fn provider(&self) -> &Self::Provider {
        &self.provider
    }

    fn preflight(&self) -> &Self::Preflight {
        self
    }

    fn validation(&self) -> &Self::Validation {
        &FIXTURE_VALIDATION
    }

    fn manifest_builder(&self) -> &Self::ManifestBuilder {
        &FIXTURE_MANIFEST
    }
}

fn proof_carry_extra_data(
    input: &GuestInput,
    namespace: Option<(&str, serde_json::Value)>,
) -> RaikoResult<Option<serde_json::Value>> {
    let mut extra_data = encode_proof_carry_data(&input.proof_carry_data)?;
    if let Some((namespace, metadata)) = namespace
        && let Some(root) = extra_data.as_object_mut()
    {
        root.insert(namespace.to_string(), metadata);
    }
    Ok(Some(extra_data))
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct FixtureRisc0Prover;

impl GuestInputCodec<GuestInput> for FixtureRisc0Prover {
    fn encode(&self, input: &GuestInput, _config: &ProverConfig) -> RaikoResult<Bytes> {
        let bytes = bincode::serialize(input)
            .map_err(|e| RaikoError::Guest(format!("Failed to serialize fixture input: {e}")))?;
        Ok(Bytes::from(bytes))
    }
}

#[async_trait::async_trait]
impl<B> Prover<B> for FixtureRisc0Prover
where
    B: ProverBackend,
{
    type GuestInput = GuestInput;

    fn encode(&self, input: &Self::GuestInput, config: &ProverConfig) -> RaikoResult<Bytes> {
        GuestInputCodec::encode(self, input, config)
    }

    async fn prove_encoded(
        &self,
        input: Bytes,
        config: &ProverConfig,
        _backend: &B,
    ) -> RaikoResult<Proof> {
        let guest_input: GuestInput = bincode::deserialize(input.as_ref())
            .map_err(|e| RaikoError::Guest(format!("Failed to deserialize fixture input: {e}")))?;
        let should_fail = config
            .get("shasta_data_sources")
            .and_then(Value::as_array)
            .is_some_and(|sources| {
                sources.iter().any(|source| {
                    source
                        .get("tx_data_from_blob")
                        .and_then(Value::as_array)
                        .is_some_and(|blobs| !blobs.is_empty())
                })
            });
        if should_fail {
            return Err(RaikoError::Guest(
                "RISC0 proposal mock execution failed: proposal mode blob usage verification failed: missing proposal source for data source index 0".to_string(),
            ));
        }

        Ok(Proof {
            proof: Some("0xfixture-risc0-proof".to_string()),
            input: Some(B256::ZERO),
            extra_data: proof_carry_extra_data(&guest_input, None)?,
            ..Default::default()
        })
    }

    async fn aggregate(
        &self,
        _input: raiko2_primitives::AggregationGuestInput,
        _config: &ProverConfig,
        _backend: &B,
    ) -> RaikoResult<Proof> {
        Ok(Proof {
            proof: Some("0xfixture-risc0-aggregation".to_string()),
            input: Some(B256::ZERO),
            ..Default::default()
        })
    }
}

#[derive(Clone)]
pub(crate) struct FixtureSp1Prover {
    config: Sp1Config,
}

impl FixtureSp1Prover {
    const fn new(config: Sp1Config) -> Self {
        Self { config }
    }

    fn resolve_config(&self, config: &ProverConfig) -> RaikoResult<Sp1Config> {
        let overrides = config
            .get("sp1")
            .cloned()
            .map(serde_json::from_value::<Sp1ConfigOverrides>)
            .transpose()
            .map_err(|e| {
                RaikoError::InvalidRequestConfig(format!("Failed to parse 'sp1' prover args: {e}"))
            })?
            .unwrap_or_default();
        let effective_config = self.config.merged_with(&overrides);
        if overrides.has_network_overrides() && effective_config.prover != ProverMode::Network {
            return Err(RaikoError::InvalidRequestConfig(
                "sp1 network-only settings require sp1.prover=network".to_string(),
            ));
        }
        effective_config
            .validate()
            .map_err(RaikoError::InvalidRequestConfig)?;
        Ok(effective_config)
    }
}

impl GuestInputCodec<GuestInput> for FixtureSp1Prover {
    fn encode(&self, input: &GuestInput, _config: &ProverConfig) -> RaikoResult<Bytes> {
        let bytes = bincode::serialize(input)
            .map_err(|e| RaikoError::Guest(format!("Failed to serialize fixture input: {e}")))?;
        Ok(Bytes::from(bytes))
    }
}

#[async_trait::async_trait]
impl<B> Prover<B> for FixtureSp1Prover
where
    B: ProverBackend,
{
    type GuestInput = GuestInput;

    fn encode(&self, input: &Self::GuestInput, config: &ProverConfig) -> RaikoResult<Bytes> {
        GuestInputCodec::encode(self, input, config)
    }

    async fn prove_encoded(
        &self,
        input: Bytes,
        config: &ProverConfig,
        _backend: &B,
    ) -> RaikoResult<Proof> {
        let effective_config = self.resolve_config(config)?;
        let guest_input: GuestInput = bincode::deserialize(input.as_ref())
            .map_err(|e| RaikoError::Guest(format!("Failed to deserialize fixture input: {e}")))?;

        match effective_config.mode {
            ExecutionMode::Execute => {
                let metadata = json!({
                    "zkvm": "sp1",
                    "mode": ExecutionMode::Execute.as_str(),
                    "public_values": alloy_primitives::hex::encode_prefixed(B256::ZERO),
                    "exit_code": 0,
                    "gas": 0,
                    "total_instruction_count": 0,
                    "total_syscall_count": 0,
                    "touched_memory_addresses": 0,
                    "cycle_tracker": [{ "label": "fixture", "cycles": 1 }],
                    "invocation_tracker": [{ "label": "fixture", "count": 1 }],
                    "opcode_counts": [],
                    "syscall_counts": [],
                });
                Ok(Proof {
                    input: Some(B256::ZERO),
                    extra_data: proof_carry_extra_data(&guest_input, Some(("sp1", metadata)))?,
                    ..Default::default()
                })
            }
            ExecutionMode::Prove => Ok(Proof {
                proof: Some("0xfixture-sp1-proof".to_string()),
                input: Some(B256::ZERO),
                extra_data: proof_carry_extra_data(&guest_input, None)?,
                ..Default::default()
            }),
        }
    }

    async fn aggregate(
        &self,
        _input: raiko2_primitives::AggregationGuestInput,
        config: &ProverConfig,
        _backend: &B,
    ) -> RaikoResult<Proof> {
        let effective_config = self.resolve_config(config)?;
        if matches!(effective_config.mode, ExecutionMode::Execute) {
            return Err(RaikoError::InvalidRequestConfig(
                "sp1.mode=execute is not supported for aggregation".to_string(),
            ));
        }

        Ok(Proof {
            proof: Some("0xfixture-sp1-aggregation".to_string()),
            input: Some(B256::ZERO),
            ..Default::default()
        })
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

    async fn batch_l1_headers(&self, block_numbers: &[u64]) -> RaikoResult<Vec<Header>> {
        let mut out = Vec::with_capacity(block_numbers.len());
        for block_number in block_numbers {
            if self.input.taiko.l1_header.number == *block_number {
                out.push(self.input.taiko.l1_header.clone());
                continue;
            }

            let header = self
                .input
                .taiko
                .l1_ancestor_headers
                .iter()
                .find(|header| header.number == *block_number)
                .cloned()
                .ok_or_else(|| {
                    RaikoError::RPC(format!(
                        "fixture missing L1 header for block {block_number}"
                    ))
                })?;
            out.push(header);
        }

        Ok(out)
    }

    async fn shasta_proposal_event(
        &self,
        _l1_contract: alloy_primitives::Address,
        l1_inclusion_block_number: u64,
        proposal_id: u64,
    ) -> RaikoResult<ShastaEventData> {
        if self.input.taiko.proposal_id != proposal_id {
            return Err(RaikoError::RPC(format!(
                "fixture missing Shasta proposal event for proposal_id {proposal_id}"
            )));
        }
        if self.input.taiko.l1_header.number != 0 {
            let expected_l1_inclusion_block_number = self.input.taiko.l1_header.number + 1;
            if expected_l1_inclusion_block_number != l1_inclusion_block_number {
                return Err(RaikoError::RPC(format!(
                    "fixture Shasta inclusion block mismatch: expected {expected_l1_inclusion_block_number}, got {l1_inclusion_block_number}"
                )));
            }
        }
        Ok(self.input.taiko.proposal_event.clone())
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
    let spec = FixtureSpec::new(
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
    let spec = FixtureSpec::new(
        PipelineKey::ShastaRisc0,
        FixtureRisc0Prover,
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
    let spec = FixtureSpec::new(
        PipelineKey::ShastaSp1,
        FixtureSp1Prover::new(Sp1Config {
            recursion: RecursionMode::Plonk,
            prover: ProverMode::Local,
            mode: ExecutionMode::Prove,
            verify: true,
            ..Sp1Config::default()
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
