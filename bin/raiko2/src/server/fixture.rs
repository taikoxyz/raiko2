//! Shared fixture-backed local server harness.

use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use alloy::consensus::Header;
use alloy_primitives::{B256, Bytes};
use anyhow::Result;
use axum::{Json, Router, routing::post};
use raiko2_engine::{Engine, EngineObserver};
#[cfg(test)]
use raiko2_pipeline::forks::shasta::{
    load_risc0_boundless_shasta_backend, load_risc0_shasta_backend, load_sp1_shasta_backend,
};
use raiko2_pipeline::{
    NativeBackend, NoopManifestBuilder, NoopValidation, PipelineKey, PipelineSpec, Preflight,
    ProverBackend, Risc0ShastaBackend, Sp1ShastaBackend, forks::shasta::load_shasta_backends,
};
use raiko2_primitives::{
    Proof, ProofContext, ProofRequest, ProofType, ProverConfig, RaikoError, RaikoResult,
};
use raiko2_primitives_shasta::{GuestInput, encode_proof_carry_data};
use raiko2_protocol_shasta::shasta::ShastaEventData;
use raiko2_prover::{
    GuestInputCodec, Prover,
    native::NativeProver,
    sp1::{
        ExecutionMode, ProverMode, RecursionMode, Sp1Config, Sp1ConfigOverrides, Sp1RequestContext,
        Sp1SystemConfig,
    },
};
use raiko2_provider::Provider;
use raiko2_queue::{MemoryStore, RetryPolicy, SchedulerConfig};
use raiko2_runtime::RuntimeManager;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tracing::info;

use super::AppState;
use super::app;
use super::net;
use super::sampling::ZkAnySampler;
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
    pub(crate) fn from_repo_shared_fixture() -> Self {
        let raw = include_str!(
            "../../../../tests/fixtures/shasta_guest_input_taiko_mainnet_proposal_2222_l2_5412225_5412416.json"
        );
        let mut input: GuestInput =
            serde_json::from_str(raw).expect("parse shared fixture json as GuestInput");
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
        if let Some(shasta) = ctx.request.shasta {
            input.taiko.prover_data.last_anchor_block_number =
                Some(shasta.last_anchor_block_number);
            input.taiko.prover_data.checkpoint =
                shasta
                    .checkpoint
                    .map(|checkpoint| raiko2_protocol_shasta::shasta::Checkpoint {
                        blockNumber: checkpoint.block_number.try_into().expect("checkpoint fits"),
                        blockHash: checkpoint.block_hash,
                        stateRoot: checkpoint.state_root,
                    });
        }
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

    fn resolve_config(
        &self,
        config: &ProverConfig,
        fallback_context: Sp1RequestContext,
    ) -> RaikoResult<Sp1Config> {
        let overrides = match config.get("sp1") {
            Some(value) => Sp1ConfigOverrides::deserialize(value).map_err(|e| {
                RaikoError::InvalidRequestConfig(format!("Failed to parse 'sp1' prover args: {e}"))
            })?,
            None => Sp1ConfigOverrides::default(),
        };
        let system = match config.get("sp1_system") {
            Some(value) => Some(Sp1SystemConfig::deserialize(value).map_err(|e| {
                RaikoError::InvalidRequestConfig(format!(
                    "Failed to parse internal 'sp1_system' config: {e}"
                ))
            })?),
            None => None,
        };
        self.config
            .resolve_request_config(Some(&overrides), fallback_context)
            .map(|config| {
                system
                    .as_ref()
                    .map_or(config.clone(), |system| system.applied_to(&config))
            })
            .map_err(|err| RaikoError::InvalidRequestConfig(err.to_string()))
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
        let effective_config = self.resolve_config(
            config,
            Sp1RequestContext::ProposalBatch { aggregate: false },
        )?;
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
        self.resolve_config(config, Sp1RequestContext::Aggregation)?;

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
    config.prover.sp1.prover = raiko2_prover::sp1::ProverMode::Local;
    config.rpc.pairs = vec![NetworkPairConfig {
        network: "taiko_dev".to_string(),
        l1_network: "ethereum".to_string(),
        l1_rpc: Some("http://localhost:8545".to_string()),
        beacon_rpc: None,
        l1_genesis_time: None,
        l1_seconds_per_slot: None,
        l2_rpc: Some("http://localhost:9545".to_string()),
        l2_provider: raiko2_provider::L2ProviderKind::Reth,
        l2_witness_rpc: None,
        sp1_verifier_rpc_url: None,
        sp1_verifier_address: None,
        boundless: crate::config::BoundlessPairConfig::default(),
    }];
    config
}

const fn memory_scheduler_config() -> SchedulerConfig {
    SchedulerConfig {
        lease_duration: Duration::from_secs(60),
        task_timeout: Duration::from_secs(60),
        retry: RetryPolicy::None,
    }
}

fn engine_observer(runtime: Arc<RuntimeManager>) -> Arc<dyn EngineObserver> {
    Arc::new(RuntimeObserver::new(
        runtime,
        "taiko_dev/ethereum".to_string(),
    ))
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

fn native_fixture_engine_with_observer(
    observer: Option<Arc<dyn EngineObserver>>,
) -> NativeFixtureEngine {
    let provider = FixtureProvider::from_repo_shared_fixture();
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
            shasta: None,
            proof_type: ProofType::Native,
            blob_proof_type: None,
            prover: None,
            graffiti: None,
        },
        raiko2_primitives::ProverConfig::default(),
    );
    build_engine_with_observer(spec, ctx, observer)
}

#[cfg(test)]
pub(crate) fn native_fixture_engine() -> NativeFixtureEngine {
    native_fixture_engine_with_observer(None)
}

#[cfg(test)]
pub(crate) fn risc0_fixture_engine(context_config: serde_json::Value) -> Risc0FixtureEngine {
    risc0_fixture_engine_with_observer(context_config, None)
}

#[cfg(test)]
fn risc0_fixture_engine_with_observer(
    context_config: serde_json::Value,
    observer: Option<Arc<dyn EngineObserver>>,
) -> Risc0FixtureEngine {
    risc0_fixture_engine_for_pipeline(context_config, PipelineKey::ShastaRisc0, observer)
}

#[cfg(test)]
fn risc0_fixture_engine_for_pipeline(
    context_config: serde_json::Value,
    pipeline_key: PipelineKey,
    observer: Option<Arc<dyn EngineObserver>>,
) -> Risc0FixtureEngine {
    let backend = load_risc0_backend_for_pipeline(pipeline_key);
    risc0_fixture_engine_for_pipeline_with_backend(context_config, pipeline_key, observer, backend)
}

fn risc0_fixture_engine_for_pipeline_with_backend(
    context_config: serde_json::Value,
    pipeline_key: PipelineKey,
    observer: Option<Arc<dyn EngineObserver>>,
    backend: Risc0ShastaBackend,
) -> Risc0FixtureEngine {
    let provider = FixtureProvider::from_repo_shared_fixture();
    let spec = FixtureSpec::new(pipeline_key, FixtureRisc0Prover, backend, provider);
    let ctx = ProofContext::new(
        ProofRequest {
            l1_chain_id: 1,
            l2_chain_id: 167_001,
            proposal_id: 0,
            l2_block_range: None,
            shasta: None,
            proof_type: ProofType::Risc0,
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

#[cfg(test)]
fn sp1_fixture_engine_with_observer(
    context_config: serde_json::Value,
    observer: Option<Arc<dyn EngineObserver>>,
) -> Sp1FixtureEngine {
    let backend = load_sp1_shasta_backend().expect("load SP1 Shasta guest ELFs");
    sp1_fixture_engine_with_backend(context_config, observer, backend)
}

fn sp1_fixture_engine_with_backend(
    context_config: serde_json::Value,
    observer: Option<Arc<dyn EngineObserver>>,
    backend: Sp1ShastaBackend,
) -> Sp1FixtureEngine {
    let provider = FixtureProvider::from_repo_shared_fixture();
    let spec = FixtureSpec::new(
        PipelineKey::ShastaSp1,
        FixtureSp1Prover::new(Sp1Config {
            recursion: RecursionMode::Plonk,
            prover: ProverMode::Local,
            mode: ExecutionMode::Prove,
            verify: true,
            ..Sp1Config::default()
        }),
        backend,
        provider,
    );
    let ctx = ProofContext::new(
        ProofRequest {
            l1_chain_id: 1,
            l2_chain_id: 167_001,
            proposal_id: 0,
            l2_block_range: None,
            shasta: None,
            proof_type: ProofType::Sp1,
            blob_proof_type: None,
            prover: None,
            graffiti: None,
        },
        context_config,
    );
    build_engine_with_observer(spec, ctx, observer)
}

#[cfg(test)]
fn load_risc0_backend_for_pipeline(pipeline_key: PipelineKey) -> Risc0ShastaBackend {
    if pipeline_key == PipelineKey::ShastaRisc0Network {
        load_risc0_boundless_shasta_backend().expect("load RISC0 Boundless Shasta guest ELFs")
    } else {
        load_risc0_shasta_backend().expect("load RISC0 Shasta guest ELFs")
    }
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
    let zk_any_sampler = Arc::new(Mutex::new(ZkAnySampler::from_config(&config.prover.zk_any)));
    AppState {
        config: Arc::new(config),
        pipelines: Arc::new(factory),
        runtime: Arc::new(
            RuntimeManager::new(unique_runtime_root("raiko2-e2e-runtime"))
                .expect("runtime manager"),
        ),
        zk_any_sampler,
    }
}

#[cfg(test)]
pub(crate) fn app_with_risc0_fixture_engine(config: Config, engine: Risc0FixtureEngine) -> Router {
    let state = app_with_engine(
        config,
        "taiko_dev/ethereum",
        PipelineKey::ShastaRisc0,
        engine,
    );
    app::build_router(state)
}

#[cfg(test)]
pub(crate) fn app_with_observed_risc0_fixture_engine(
    config: Config,
) -> (Router, Risc0FixtureEngine) {
    let runtime = Arc::new(
        RuntimeManager::new(unique_runtime_root("raiko2-e2e-observed-runtime"))
            .expect("runtime manager"),
    );
    let observer = engine_observer(Arc::clone(&runtime));
    let engine = risc0_fixture_engine_with_observer(json!({}), Some(observer));

    let mut factory = StaticPipelineFactory::default();
    factory.insert(
        "taiko_dev/ethereum".to_string(),
        PipelineKey::ShastaRisc0,
        Arc::new(engine.clone()),
    );
    let zk_any_sampler = Arc::new(Mutex::new(ZkAnySampler::from_config(&config.prover.zk_any)));
    let state = AppState {
        config: Arc::new(config),
        pipelines: Arc::new(factory),
        runtime,
        zk_any_sampler,
    };

    (app::build_router(state), engine)
}

#[cfg(test)]
pub(crate) fn state_with_observed_sp1_fixture_engine(
    config: Config,
) -> (AppState, Sp1FixtureEngine) {
    let runtime = Arc::new(
        RuntimeManager::new(unique_runtime_root("raiko2-e2e-observed-sp1-runtime"))
            .expect("runtime manager"),
    );
    let observer = engine_observer(Arc::clone(&runtime));
    let engine = sp1_fixture_engine_with_observer(json!({}), Some(observer));

    let mut factory = StaticPipelineFactory::default();
    factory.insert(
        "taiko_dev/ethereum".to_string(),
        PipelineKey::ShastaSp1,
        Arc::new(engine.clone()),
    );
    let zk_any_sampler = Arc::new(Mutex::new(ZkAnySampler::from_config(&config.prover.zk_any)));
    let state = AppState {
        config: Arc::new(config),
        pipelines: Arc::new(factory),
        runtime,
        zk_any_sampler,
    };

    (state, engine)
}

#[cfg(test)]
pub(crate) fn app_with_observed_sp1_fixture_engine(config: Config) -> (Router, Sp1FixtureEngine) {
    let (state, engine) = state_with_observed_sp1_fixture_engine(config);
    (app::build_router(state), engine)
}

#[cfg(test)]
pub(crate) fn app_with_observed_native_fixture_engine(
    config: Config,
) -> (Router, NativeFixtureEngine) {
    let runtime = Arc::new(
        RuntimeManager::new(unique_runtime_root("raiko2-e2e-observed-native-runtime"))
            .expect("runtime manager"),
    );
    let observer = engine_observer(Arc::clone(&runtime));
    let engine = native_fixture_engine_with_observer(Some(observer));

    let mut factory = StaticPipelineFactory::default();
    factory.insert(
        "taiko_dev/ethereum".to_string(),
        PipelineKey::ShastaNative,
        Arc::new(engine.clone()),
    );
    let zk_any_sampler = Arc::new(Mutex::new(ZkAnySampler::from_config(&config.prover.zk_any)));
    let state = AppState {
        config: Arc::new(config),
        pipelines: Arc::new(factory),
        runtime,
        zk_any_sampler,
    };

    (app::build_router(state), engine)
}

#[cfg(test)]
pub(crate) fn app_with_observed_risc0_boundless_fixture_engine(
    config: Config,
) -> (Router, Risc0FixtureEngine) {
    let runtime = Arc::new(
        RuntimeManager::new(unique_runtime_root(
            "raiko2-e2e-observed-risc0-boundless-runtime",
        ))
        .expect("runtime manager"),
    );
    let observer = engine_observer(Arc::clone(&runtime));
    let engine = risc0_fixture_engine_for_pipeline(
        json!({}),
        PipelineKey::ShastaRisc0Network,
        Some(observer),
    );

    let mut factory = StaticPipelineFactory::default();
    factory.insert(
        "taiko_dev/ethereum".to_string(),
        PipelineKey::ShastaRisc0Network,
        Arc::new(engine.clone()),
    );
    let zk_any_sampler = Arc::new(Mutex::new(ZkAnySampler::from_config(&config.prover.zk_any)));
    let state = AppState {
        config: Arc::new(config),
        pipelines: Arc::new(factory),
        runtime,
        zk_any_sampler,
    };

    (app::build_router(state), engine)
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
    let zk_any_sampler = Arc::new(Mutex::new(ZkAnySampler::from_config(&config.prover.zk_any)));
    let observer = engine_observer(Arc::clone(&runtime));
    let maintenance_interval = Duration::from_millis(config.queue.maintenance_interval_ms);
    let workers = config.queue.workers;
    let shasta_backends = load_shasta_backends().map_err(anyhow::Error::msg)?;

    let mut factory = StaticPipelineFactory::default();

    let native_engine = native_fixture_engine_with_observer(Some(Arc::clone(&observer)));
    native_engine.start_workers_with_maintenance_interval(workers, maintenance_interval);
    factory.insert(
        "taiko_dev/ethereum".to_string(),
        PipelineKey::ShastaNative,
        Arc::new(native_engine),
    );

    let risc0_engine = risc0_fixture_engine_for_pipeline_with_backend(
        json!({}),
        PipelineKey::ShastaRisc0,
        Some(Arc::clone(&observer)),
        shasta_backends.risc0,
    );
    risc0_engine.start_workers_with_maintenance_interval(workers, maintenance_interval);
    factory.insert(
        "taiko_dev/ethereum".to_string(),
        PipelineKey::ShastaRisc0,
        Arc::new(risc0_engine),
    );

    let sp1_engine =
        sp1_fixture_engine_with_backend(json!({}), Some(observer), shasta_backends.sp1);
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
        zk_any_sampler,
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
