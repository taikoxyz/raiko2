//! SP1 zkVM Prover for Raiko V2
//!
//! This module provides the SP1 prover implementation for generating
//! zero-knowledge proofs of Taiko block execution.

mod types;

pub use types::{
    ExecutionMode, ProverMode, RecursionMode, Sp1Config, Sp1ConfigError, Sp1ConfigOverrides,
    Sp1ExecutionMetadata, Sp1FulfillmentStrategy, Sp1NetworkMetadata, Sp1NetworkMode,
    Sp1NetworkSubmissionProgress, Sp1RemoteVerifyConfig, Sp1RequestContext, Sp1Response,
    Sp1SystemConfig,
};

use alloy::{providers::ProviderBuilder, sol};
use alloy_primitives::{Address, B256, Bytes};
use raiko2_pipeline::{ProofStage, ProverBackend};
use raiko2_primitives::{AggregationGuestInput, Proof, ProverConfig, RaikoError, RaikoResult};
use raiko2_primitives_shasta::{GuestInput, ShastaZkAggregationGuestInput};
use serde::Deserialize;
use sp1_sdk::{
    CpuProver, HashableKey, NetworkProver, Prover as _, ProverClient, SP1Proof, SP1ProofMode,
    SP1ProofWithPublicValues, SP1ProvingKey, SP1Stdin, SP1VerifyingKey,
    network::Error as Sp1NetworkError,
};
use std::str::FromStr;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::time::timeout;
use tracing::info;
use url::Url;

use crate::{
    GuestInputCodec, ProverProgress, ProverProgressObserver, build_shasta_aggregation_input,
    parse_shasta_aggregation_input_hash, parse_shasta_proposal_input_hash, with_shasta_extra_data,
};

const SP1_NETWORK_WAIT_RETRY_DELAY: Duration = Duration::from_secs(15);
const SP1_NETWORK_REQUEST_RETRY_DELAY: Duration = Duration::from_secs(15);

sol!(
    #[sol(rpc)]
    #[allow(dead_code)]
    contract ISP1Verifier {
        function verifyProof(
            bytes32 programVKey,
            bytes publicValues,
            bytes proofBytes
        ) external view;
    }
);

/// SP1 Prover for Shasta proposal proofs.
#[derive(Clone)]
pub struct Sp1Prover {
    config: Sp1Config,
    setup_cache: Arc<Sp1SetupCache>,
}

struct Sp1SetupCache {
    proposal: OnceLock<Arc<Sp1ProgramSetup>>,
    aggregation: OnceLock<Arc<Sp1ProgramSetup>>,
}

#[derive(Clone)]
struct Sp1ProgramSetup {
    pk: SP1ProvingKey,
    vk: SP1VerifyingKey,
}

#[must_use]
/// Returns a stable JSON representation of an SP1 verifying key.
///
/// # Panics
///
/// Panics if the verifying key cannot be serialized to JSON.
pub fn sp1_vk_uuid(vk: &SP1VerifyingKey) -> String {
    serde_json::to_string(vk).expect("SP1 verifying key should serialize")
}

#[must_use]
pub fn sp1_vk_digest(vk: &SP1VerifyingKey) -> String {
    alloy_primitives::hex::encode_prefixed(vk.hash_bytes())
}

/// Parses either an SP1 verifying key JSON string or a 32-byte hex image id.
///
/// SP1 verifying key JSON is kept in SP1's native `hash_u32` word form. Raw 32-byte
/// image ids are encoded into little-endian `u32` words so RISC0/Boundless aggregation
/// paths can round-trip them back to their original bytes.
///
/// # Errors
///
/// Returns an error when the input is neither valid SP1 verifying key JSON nor a 32-byte hex
/// payload.
pub fn sp1_image_id_words_from_uuid(raw: &str) -> Result<[u32; 8], String> {
    if let Ok(vk) = serde_json::from_str::<SP1VerifyingKey>(raw) {
        return Ok(vk.hash_u32());
    }

    let bytes =
        alloy_primitives::hex::decode(raw).map_err(|err| format!("invalid hex uuid: {err}"))?;
    if bytes.len() != 32 {
        return Err(format!(
            "expected 32 bytes or SP1 verifying key JSON, got {}",
            bytes.len()
        ));
    }

    let mut words = [0u32; 8];
    for (index, chunk) in bytes.chunks_exact(4).enumerate() {
        let mut word = [0u8; 4];
        word.copy_from_slice(chunk);
        words[index] = u32::from_le_bytes(word);
    }
    Ok(words)
}

impl Sp1Prover {
    /// Create a new SP1 prover with the given configuration.
    ///
    /// Call `preload_setup` before using this prover to prove. Server code should prefer
    /// `new_with_backend` so SP1 setup happens during startup instead of the async proving path.
    #[must_use]
    pub fn new(config: Sp1Config) -> Self {
        Self {
            config,
            setup_cache: Arc::new(Sp1SetupCache::new()),
        }
    }

    /// Create a new SP1 prover and eagerly prepare proving/verifying keys for the configured ELF
    /// backend.
    ///
    /// # Errors
    ///
    /// Returns an error if the backend cannot provide the configured proposal or aggregation ELF.
    pub fn new_with_backend<B>(config: Sp1Config, backend: &B) -> RaikoResult<Self>
    where
        B: ProverBackend,
    {
        let prover = Self::new(config);
        prover.preload_setup(backend)?;
        Ok(prover)
    }

    /// Eagerly prepare proving/verifying keys for both SP1 proof stages.
    ///
    /// # Errors
    ///
    /// Returns an error if the backend cannot provide either ELF.
    pub fn preload_setup<B>(&self, backend: &B) -> RaikoResult<()>
    where
        B: ProverBackend,
    {
        self.preload_setup_for_stage(backend, ProofStage::Proposal)?;
        self.preload_setup_for_stage(backend, ProofStage::Aggregation)?;
        Ok(())
    }

    fn resolve_config_for_request(
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
        let effective = self
            .config
            .resolve_request_config(Some(&overrides), fallback_context)
            .map_err(|err| RaikoError::InvalidRequestConfig(err.to_string()))?;
        Ok(system
            .as_ref()
            .map_or(effective.clone(), |system| system.applied_to(&effective)))
    }

    fn preload_setup_for_stage<B>(
        &self,
        backend: &B,
        stage: ProofStage,
    ) -> RaikoResult<Arc<Sp1ProgramSetup>>
    where
        B: ProverBackend,
    {
        let cell = self.setup_cache.cell(stage);
        if let Some(setup) = cell.get() {
            return Ok(Arc::clone(setup));
        }

        let elf = backend.elf(stage)?;
        let client = ProverClient::builder().cpu().build();
        let (pk, vk) = client.setup(elf);
        let setup = Arc::new(Sp1ProgramSetup { pk, vk });

        match cell.set(Arc::clone(&setup)) {
            Ok(()) => {
                tracing::info!(
                    stage = sp1_stage_name(stage),
                    vkey_hash = %sp1_vk_digest(&setup.vk),
                    "Initialized SP1 program setup"
                );
                Ok(setup)
            }
            Err(setup) => Ok(cell.get().cloned().unwrap_or(setup)),
        }
    }

    fn cached_setup_for_stage(&self, stage: ProofStage) -> RaikoResult<Arc<Sp1ProgramSetup>> {
        self.setup_cache.cell(stage).get().cloned().ok_or_else(|| {
            RaikoError::InvalidRequestConfig(format!(
                "SP1 {} setup is not initialized; call Sp1Prover::preload_setup before proving",
                sp1_stage_name(stage)
            ))
        })
    }
}

impl Sp1SetupCache {
    const fn new() -> Self {
        Self {
            proposal: OnceLock::new(),
            aggregation: OnceLock::new(),
        }
    }

    const fn cell(&self, stage: ProofStage) -> &OnceLock<Arc<Sp1ProgramSetup>> {
        match stage {
            ProofStage::Proposal => &self.proposal,
            ProofStage::Aggregation => &self.aggregation,
        }
    }
}

const fn sp1_stage_name(stage: ProofStage) -> &'static str {
    match stage {
        ProofStage::Proposal => "proposal",
        ProofStage::Aggregation => "aggregation",
    }
}

impl GuestInputCodec<GuestInput> for Sp1Prover {
    fn encode(&self, input: &GuestInput, _config: &ProverConfig) -> RaikoResult<Bytes> {
        let bytes = bincode::serialize(input)
            .map_err(|e| RaikoError::Guest(format!("Failed to serialize input: {e}")))?;
        Ok(Bytes::from(bytes))
    }
}

#[async_trait::async_trait]
impl<B> crate::Prover<B> for Sp1Prover
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
        backend: &B,
    ) -> RaikoResult<Proof> {
        self.prove_encoded_with_observer(input, config, backend, None)
            .await
    }

    async fn prove_encoded_with_observer(
        &self,
        input: Bytes,
        config: &ProverConfig,
        backend: &B,
        observer: Option<std::sync::Arc<dyn ProverProgressObserver>>,
    ) -> RaikoResult<Proof> {
        let effective_config = self.resolve_config_for_request(
            config,
            Sp1RequestContext::ProposalBatch { aggregate: false },
        )?;
        info!(mode = %effective_config.mode.as_str(), "Starting SP1 proposal run...");

        let guest_input: GuestInput = bincode::deserialize(input.as_ref())
            .map_err(|e| RaikoError::Guest(format!("Failed to deserialize input: {e}")))?;

        // Use the proposal ELF selected by the configured backend.
        match effective_config.mode {
            ExecutionMode::Execute => {
                if effective_config.prover == ProverMode::Network {
                    return Err(RaikoError::InvalidRequestConfig(
                        "sp1.mode=execute does not support sp1.prover=network".to_string(),
                    ));
                }
                execute_proposal_with_local_client(
                    effective_config.prover,
                    backend.elf(ProofStage::Proposal)?.to_vec(),
                    guest_input,
                )
                .await
            }
            ExecutionMode::Prove => {
                let proof_mode: SP1ProofMode = effective_config.recursion.into();
                let setup = self.cached_setup_for_stage(ProofStage::Proposal)?;
                match effective_config.prover {
                    ProverMode::Mock | ProverMode::Local => {
                        prove_proposal_with_local_client(
                            effective_config.prover,
                            setup,
                            guest_input,
                            proof_mode,
                            effective_config.verify,
                        )
                        .await
                    }
                    ProverMode::Network => {
                        let mut stdin = SP1Stdin::new();
                        stdin.write(&guest_input);
                        let client = build_network_prover(&effective_config)?;
                        prove_proposal_with_network_client(
                            &client,
                            setup.as_ref(),
                            stdin,
                            proof_mode,
                            &effective_config,
                            &guest_input,
                            observer,
                        )
                        .await
                    }
                }
            }
        }
    }

    async fn aggregate(
        &self,
        input: AggregationGuestInput,
        config: &ProverConfig,
        _backend: &B,
    ) -> RaikoResult<Proof> {
        let effective_config =
            self.resolve_config_for_request(config, Sp1RequestContext::Aggregation)?;
        info!(
            "Starting SP1 aggregation proof generation with {} proofs...",
            input.proofs.len()
        );

        let aggregation_input = build_shasta_aggregation_input(&input.proofs)?;

        // Get the proposal prover's verifying key for proof verification.
        // The proposal proofs were generated with the proposal ELF.
        let proposal_setup = self.cached_setup_for_stage(ProofStage::Proposal)?;
        let aggregation_setup = self.cached_setup_for_stage(ProofStage::Aggregation)?;
        let proof_mode: SP1ProofMode = effective_config.recursion.into();

        match effective_config.prover {
            ProverMode::Mock | ProverMode::Local => {
                aggregate_with_local_client(
                    effective_config.prover,
                    proposal_setup,
                    aggregation_setup,
                    input,
                    aggregation_input,
                    proof_mode,
                    effective_config.verify,
                )
                .await
            }
            ProverMode::Network => {
                // The guest reads ShastaZkAggregationGuestInput via sp1_zkvm::io::read().
                let mut stdin = SP1Stdin::new();
                stdin.write(&aggregation_input);
                let client = build_network_prover(&effective_config)?;
                aggregate_with_network_client(
                    &client,
                    proposal_setup.as_ref(),
                    aggregation_setup.as_ref(),
                    &input,
                    stdin,
                    &effective_config,
                    None,
                )
                .await
            }
        }
    }

    async fn aggregate_with_observer(
        &self,
        input: AggregationGuestInput,
        config: &ProverConfig,
        _backend: &B,
        observer: Option<std::sync::Arc<dyn ProverProgressObserver>>,
    ) -> RaikoResult<Proof> {
        let effective_config =
            self.resolve_config_for_request(config, Sp1RequestContext::Aggregation)?;
        info!(
            "Starting SP1 aggregation proof generation with {} proofs...",
            input.proofs.len()
        );

        let aggregation_input = build_shasta_aggregation_input(&input.proofs)?;
        let proposal_setup = self.cached_setup_for_stage(ProofStage::Proposal)?;
        let aggregation_setup = self.cached_setup_for_stage(ProofStage::Aggregation)?;
        let proof_mode: SP1ProofMode = effective_config.recursion.into();

        match effective_config.prover {
            ProverMode::Mock | ProverMode::Local => {
                aggregate_with_local_client(
                    effective_config.prover,
                    proposal_setup,
                    aggregation_setup,
                    input,
                    aggregation_input,
                    proof_mode,
                    effective_config.verify,
                )
                .await
            }
            ProverMode::Network => {
                let mut stdin = SP1Stdin::new();
                stdin.write(&aggregation_input);
                let client = build_network_prover(&effective_config)?;
                aggregate_with_network_client(
                    &client,
                    proposal_setup.as_ref(),
                    aggregation_setup.as_ref(),
                    &input,
                    stdin,
                    &effective_config,
                    observer,
                )
                .await
            }
        }
    }
}

#[derive(Clone)]
struct NetworkProofRequestResult {
    request_id: String,
    proof: SP1ProofWithPublicValues,
}

fn remote_verifier_program_vkey(vk: &SP1VerifyingKey) -> B256 {
    B256::from_slice(&vk.bytes32_raw())
}

const fn sp1_proof_mode_name(proof: &SP1ProofWithPublicValues) -> &'static str {
    match &proof.proof {
        SP1Proof::Core(_) => "core",
        SP1Proof::Compressed(_) => "compressed",
        SP1Proof::Plonk(_) => "plonk",
        SP1Proof::Groth16(_) => "groth16",
    }
}

fn encode_sp1_onchain_payload(segments: &[String], proof_bytes: &[u8]) -> String {
    let mut payload = String::from("0x");
    for segment in segments {
        payload.push_str(segment.strip_prefix("0x").unwrap_or(segment));
    }
    payload.push_str(&alloy_primitives::hex::encode(proof_bytes));
    payload
}

#[must_use]
pub fn encode_sp1_proposal_proof_payload(
    proof: &SP1ProofWithPublicValues,
    vk: &SP1VerifyingKey,
) -> Option<String> {
    let proof_bytes = remote_verifier_proof_bytes(proof)?;
    Some(encode_sp1_onchain_payload(&[vk.bytes32()], &proof_bytes))
}

#[must_use]
pub fn encode_sp1_aggregation_proof_payload(
    proof: &SP1ProofWithPublicValues,
    aggregation_vk: &SP1VerifyingKey,
    block_vk: &SP1VerifyingKey,
) -> Option<String> {
    let proof_bytes = remote_verifier_proof_bytes(proof)?;
    Some(encode_sp1_onchain_payload(
        &[
            aggregation_vk.bytes32(),
            alloy_primitives::hex::encode_prefixed(block_vk.hash_bytes()),
        ],
        &proof_bytes,
    ))
}

/// # Errors
///
/// Returns an error when the proof does not contain a valid SP1 quote or legacy encoded proof
/// payload that can be deserialized for aggregation.
pub fn load_sp1_subproof_for_aggregation(proof: &Proof) -> RaikoResult<SP1Proof> {
    if let Some(quote) = proof.quote.as_deref() {
        return serde_json::from_str::<SP1Proof>(quote)
            .map_err(|e| RaikoError::Guest(format!("Failed to deserialize SP1 proof quote: {e}")));
    }

    if let Some(proof_hex) = proof.proof.as_deref() {
        let proof_bytes = alloy_primitives::hex::decode(proof_hex)
            .map_err(|e| RaikoError::Guest(format!("Failed to decode proof hex: {e}")))?;
        let legacy: SP1ProofWithPublicValues = bincode::deserialize(&proof_bytes).map_err(|e| {
            RaikoError::Guest(format!(
                "Failed to deserialize legacy SP1ProofWithPublicValues: {e}"
            ))
        })?;
        return Ok(legacy.proof);
    }

    Err(RaikoError::Guest(
        "Aggregation requires SP1 quote or legacy proof bytes".to_string(),
    ))
}

// Old `raiko` only ran remote contract verification when the proof could be encoded into
// onchain verifier bytes. Shasta proposal proofs used for aggregation are `Compressed`, so they
// must skip this path.
fn remote_verifier_proof_bytes(proof: &SP1ProofWithPublicValues) -> Option<Vec<u8>> {
    match &proof.proof {
        SP1Proof::Plonk(_) | SP1Proof::Groth16(_) => {
            let proof_bytes = proof.bytes();
            (!proof_bytes.is_empty()).then_some(proof_bytes)
        }
        SP1Proof::Core(_) | SP1Proof::Compressed(_) => None,
    }
}

async fn verify_network_proposal_proof(
    config: &Sp1Config,
    proof: &SP1ProofWithPublicValues,
    vk: &SP1VerifyingKey,
) -> RaikoResult<()> {
    let Some(proof_bytes) = remote_verifier_proof_bytes(proof) else {
        tracing::info!(
            proof_mode = sp1_proof_mode_name(proof),
            "Skipping SP1 proposal remote verification for non-onchain proof mode"
        );
        return Ok(());
    };
    let input_hash = parse_shasta_proposal_input_hash(proof.public_values.as_slice())?;
    verify_sp1_remote_contract(
        config,
        vk,
        input_hash.as_slice().to_vec(),
        proof_bytes,
        "proposal",
    )
    .await
}

async fn verify_network_aggregation_proof(
    config: &Sp1Config,
    proof: &SP1ProofWithPublicValues,
    vk: &SP1VerifyingKey,
) -> RaikoResult<()> {
    let Some(proof_bytes) = remote_verifier_proof_bytes(proof) else {
        tracing::info!(
            proof_mode = sp1_proof_mode_name(proof),
            "Skipping SP1 aggregation remote verification for non-onchain proof mode"
        );
        return Ok(());
    };
    verify_sp1_remote_contract(
        config,
        vk,
        proof.public_values.as_slice().to_vec(),
        proof_bytes,
        "aggregation",
    )
    .await
}

async fn verify_sp1_remote_contract(
    config: &Sp1Config,
    vk: &SP1VerifyingKey,
    public_values: Vec<u8>,
    proof_bytes: Vec<u8>,
    stage: &str,
) -> RaikoResult<()> {
    let remote_verify = config.remote_verify.as_ref().ok_or_else(|| {
        RaikoError::InvalidRequestConfig(
            "sp1.prover=network with sp1.verify=true requires internal remote verifier configuration"
                .to_string(),
        )
    })?;
    let rpc_url = Url::parse(&remote_verify.rpc_url).map_err(|e| {
        RaikoError::InvalidRequestConfig(format!(
            "invalid SP1 remote verifier rpc_url '{}': {e}",
            remote_verify.rpc_url
        ))
    })?;
    let verifier_address = Address::from_str(&remote_verify.verifier_address).map_err(|e| {
        RaikoError::InvalidRequestConfig(format!(
            "invalid SP1 remote verifier address '{}': {e}",
            remote_verify.verifier_address
        ))
    })?;
    let provider = ProviderBuilder::new().connect_http(rpc_url);
    let verifier = ISP1Verifier::new(verifier_address, provider);
    let verify_timeout = Duration::from_secs(config.timeout_secs);
    let verify_started = std::time::Instant::now();

    tracing::info!(
        stage,
        rpc_url = %remote_verify.rpc_url,
        verifier_address = %remote_verify.verifier_address,
        timeout_secs = config.timeout_secs,
        "Starting SP1 remote verification"
    );

    timeout(
        verify_timeout,
        verifier
            .verifyProof(
                remote_verifier_program_vkey(vk),
                Bytes::from(public_values),
                Bytes::from(proof_bytes),
            )
            .call(),
    )
    .await
    .map_err(|_| {
        tracing::error!(
            stage,
            elapsed_ms = verify_started.elapsed().as_millis(),
            "Timed out while remotely verifying SP1 proof"
        );
        RaikoError::Guest(format!(
            "SP1 {stage} remote verification timed out after {}s",
            config.timeout_secs
        ))
    })?
    .map_err(|e| {
        tracing::error!("Failed to remotely verify SP1 {stage} proof: {:?}", e);
        RaikoError::Guest(format!("SP1 {stage} remote verification failed: {e}"))
    })?;

    tracing::info!(
        stage,
        elapsed_ms = verify_started.elapsed().as_millis(),
        "SP1 remote verification succeeded"
    );

    Ok(())
}

fn blocking_sp1_join_error(operation: &str, err: &tokio::task::JoinError) -> RaikoError {
    RaikoError::Guest(format!("SP1 {operation} task join failed: {err}"))
}

fn local_sp1_client(prover_mode: ProverMode) -> CpuProver {
    match prover_mode {
        ProverMode::Mock => ProverClient::builder().mock().build(),
        ProverMode::Local => ProverClient::builder().cpu().build(),
        ProverMode::Network => unreachable!("network prover is handled by the network path"),
    }
}

async fn execute_proposal_with_local_client(
    prover_mode: ProverMode,
    elf: Vec<u8>,
    guest_input: GuestInput,
) -> RaikoResult<Proof> {
    tokio::task::spawn_blocking(move || {
        let mut stdin = SP1Stdin::new();
        stdin.write(&guest_input);

        let client = local_sp1_client(prover_mode);
        let (public_values, execution_report) =
            client.execute(&elf, &stdin).run().map_err(|e| {
                tracing::error!("Failed to execute SP1 proposal: {:?}", e);
                RaikoError::Guest(format!("SP1 proposal execute failed: {e}"))
            })?;
        let public_values_raw = public_values.raw();
        let input_hash = parse_shasta_proposal_input_hash(public_values.as_slice())?;
        let metadata = serde_json::to_value(Sp1ExecutionMetadata::from_execution_report(
            public_values_raw,
            &execution_report,
        ))
        .map_err(|e| {
            RaikoError::Guest(format!("Failed to serialize SP1 execution metadata: {e}"))
        })?;

        Ok(Sp1Response {
            proof: None,
            vkey_hash: None,
            input: input_hash,
            sp1_proof: None,
            vkey: None,
            extra_data: with_shasta_extra_data(
                &guest_input.proof_carry_data,
                "sp1",
                Some(metadata),
            )?,
        }
        .into())
    })
    .await
    .map_err(|err| blocking_sp1_join_error("proposal execute", &err))?
}

async fn prove_proposal_with_local_client(
    prover_mode: ProverMode,
    setup: Arc<Sp1ProgramSetup>,
    guest_input: GuestInput,
    proof_mode: SP1ProofMode,
    verify: bool,
) -> RaikoResult<Proof> {
    tokio::task::spawn_blocking(move || {
        let mut stdin = SP1Stdin::new();
        stdin.write(&guest_input);

        let client = local_sp1_client(prover_mode);
        prove_proposal_with_client(
            &client,
            setup.as_ref(),
            &stdin,
            proof_mode,
            verify,
            &guest_input,
        )
    })
    .await
    .map_err(|err| blocking_sp1_join_error("proposal proof", &err))?
}

fn prove_proposal_with_client(
    client: &CpuProver,
    setup: &Sp1ProgramSetup,
    stdin: &SP1Stdin,
    proof_mode: SP1ProofMode,
    verify: bool,
    guest_input: &GuestInput,
) -> RaikoResult<Proof> {
    let proof = client
        .prove(&setup.pk, stdin)
        .mode(proof_mode)
        .run()
        .map_err(|e| {
            tracing::error!("Failed to generate SP1 proposal proof: {:?}", e);
            RaikoError::Guest(format!("SP1 proposal proof generation failed: {e}"))
        })?;

    if verify {
        client.verify(&proof, &setup.vk).map_err(|e| {
            tracing::error!("Failed to verify SP1 proposal proof: {:?}", e);
            RaikoError::Guest(format!("SP1 proposal proof verification failed: {e}"))
        })?;
    }

    let public_values = proof.public_values.as_slice();
    let input_hash = parse_shasta_proposal_input_hash(public_values)?;

    Ok(Sp1Response {
        proof: encode_sp1_proposal_proof_payload(&proof, &setup.vk),
        vkey_hash: Some(sp1_vk_digest(&setup.vk)),
        input: input_hash,
        sp1_proof: Some(proof),
        vkey: Some(setup.vk.clone()),
        extra_data: with_shasta_extra_data(&guest_input.proof_carry_data, "sp1", None)?,
    }
    .into())
}

async fn prove_proposal_with_network_client(
    client: &NetworkProver,
    setup: &Sp1ProgramSetup,
    stdin: SP1Stdin,
    proof_mode: SP1ProofMode,
    config: &Sp1Config,
    guest_input: &GuestInput,
    observer: Option<std::sync::Arc<dyn ProverProgressObserver>>,
) -> RaikoResult<Proof> {
    let request = request_network_proof(
        client, &setup.pk, stdin, proof_mode, config, observer, "proposal",
    )
    .await?;

    if config.verify {
        verify_network_proposal_proof(config, &request.proof, &setup.vk).await?;
    }

    let public_values = request.proof.public_values.as_slice();
    let input_hash = parse_shasta_proposal_input_hash(public_values)?;
    let base_extra_data = with_shasta_extra_data(&guest_input.proof_carry_data, "sp1", None)?;
    let network_metadata =
        serde_json::to_value(Sp1NetworkMetadata::from_config(request.request_id, config))
            .map_err(|e| RaikoError::Guest(format!("Failed to serialize SP1 metadata: {e}")))?;

    Ok(Sp1Response {
        proof: encode_sp1_proposal_proof_payload(&request.proof, &setup.vk),
        vkey_hash: Some(sp1_vk_digest(&setup.vk)),
        input: input_hash,
        sp1_proof: Some(request.proof),
        vkey: Some(setup.vk.clone()),
        extra_data: insert_sp1_metadata(
            base_extra_data,
            serde_json::json!({ "network": network_metadata }),
        )?,
    }
    .into())
}

async fn aggregate_with_local_client(
    prover_mode: ProverMode,
    proposal_setup: Arc<Sp1ProgramSetup>,
    aggregation_setup: Arc<Sp1ProgramSetup>,
    input: AggregationGuestInput,
    aggregation_input: ShastaZkAggregationGuestInput,
    proof_mode: SP1ProofMode,
    verify: bool,
) -> RaikoResult<Proof> {
    tokio::task::spawn_blocking(move || {
        let mut stdin = SP1Stdin::new();
        stdin.write(&aggregation_input);

        let client = local_sp1_client(prover_mode);
        aggregate_with_client(
            &client,
            proposal_setup.as_ref(),
            aggregation_setup.as_ref(),
            &input,
            stdin,
            proof_mode,
            verify,
        )
    })
    .await
    .map_err(|err| blocking_sp1_join_error("aggregation proof", &err))?
}

fn aggregate_with_client(
    client: &CpuProver,
    proposal_setup: &Sp1ProgramSetup,
    aggregation_setup: &Sp1ProgramSetup,
    input: &AggregationGuestInput,
    mut stdin: SP1Stdin,
    proof_mode: SP1ProofMode,
    verify: bool,
) -> RaikoResult<Proof> {
    for proof in &input.proofs {
        match load_sp1_subproof_for_aggregation(proof)? {
            SP1Proof::Compressed(reduce_proof) => {
                stdin.write_proof(*reduce_proof, proposal_setup.vk.vk.clone());
            }
            _ => {
                return Err(RaikoError::Guest(
                    "Aggregation requires Compressed proofs".to_string(),
                ));
            }
        }
    }

    let proof = client
        .prove(&aggregation_setup.pk, &stdin)
        .mode(proof_mode)
        .run()
        .map_err(|e| {
            tracing::error!("Failed to generate SP1 aggregation proof: {:?}", e);
            RaikoError::Guest(format!("SP1 aggregation proof generation failed: {e}"))
        })?;

    if verify {
        client.verify(&proof, &aggregation_setup.vk).map_err(|e| {
            tracing::error!("Failed to verify SP1 aggregation proof: {:?}", e);
            RaikoError::Guest(format!("SP1 aggregation proof verification failed: {e}"))
        })?;
    }

    let public_values = proof.public_values.as_slice();
    let agg_input_hash = parse_shasta_aggregation_input_hash(public_values)?;

    Ok(Sp1Response {
        proof: encode_sp1_aggregation_proof_payload(
            &proof,
            &aggregation_setup.vk,
            &proposal_setup.vk,
        ),
        vkey_hash: Some(sp1_vk_digest(&aggregation_setup.vk)),
        input: agg_input_hash,
        sp1_proof: Some(proof),
        vkey: Some(aggregation_setup.vk.clone()),
        extra_data: None,
    }
    .into())
}

async fn aggregate_with_network_client(
    client: &NetworkProver,
    proposal_setup: &Sp1ProgramSetup,
    aggregation_setup: &Sp1ProgramSetup,
    input: &AggregationGuestInput,
    mut stdin: SP1Stdin,
    config: &Sp1Config,
    observer: Option<std::sync::Arc<dyn ProverProgressObserver>>,
) -> RaikoResult<Proof> {
    for proof in &input.proofs {
        match load_sp1_subproof_for_aggregation(proof)? {
            SP1Proof::Compressed(reduce_proof) => {
                stdin.write_proof(*reduce_proof, proposal_setup.vk.vk.clone());
            }
            _ => {
                return Err(RaikoError::Guest(
                    "Aggregation requires Compressed proofs".to_string(),
                ));
            }
        }
    }

    let proof_mode: SP1ProofMode = config.recursion.into();
    let request = request_network_proof(
        client,
        &aggregation_setup.pk,
        stdin,
        proof_mode,
        config,
        observer,
        "aggregation",
    )
    .await?;

    if config.verify {
        verify_network_aggregation_proof(config, &request.proof, &aggregation_setup.vk).await?;
    }

    let public_values = request.proof.public_values.as_slice();
    let agg_input_hash = parse_shasta_aggregation_input_hash(public_values)?;
    let network_metadata =
        serde_json::to_value(Sp1NetworkMetadata::from_config(request.request_id, config))
            .map_err(|e| RaikoError::Guest(format!("Failed to serialize SP1 metadata: {e}")))?;

    Ok(Sp1Response {
        proof: encode_sp1_aggregation_proof_payload(
            &request.proof,
            &aggregation_setup.vk,
            &proposal_setup.vk,
        ),
        vkey_hash: Some(sp1_vk_digest(&aggregation_setup.vk)),
        input: agg_input_hash,
        sp1_proof: Some(request.proof),
        vkey: Some(aggregation_setup.vk.clone()),
        extra_data: insert_sp1_metadata(None, serde_json::json!({ "network": network_metadata }))?,
    }
    .into())
}

fn build_network_prover(config: &Sp1Config) -> RaikoResult<NetworkProver> {
    let private_key = std::env::var("NETWORK_PRIVATE_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            RaikoError::InvalidRequestConfig(
                "NETWORK_PRIVATE_KEY must be set for sp1.prover=network".to_string(),
            )
        })?;
    let builder = ProverClient::builder().network().private_key(&private_key);
    let builder = if let Some(rpc_url) = sp1_network_rpc_url(config)? {
        builder.rpc_url(&rpc_url)
    } else {
        builder
    };
    Ok(builder.build())
}

fn sp1_network_rpc_url(config: &Sp1Config) -> RaikoResult<Option<String>> {
    let env_rpc_url = std::env::var("NETWORK_RPC_URL").ok();
    resolve_sp1_network_rpc_url(config, env_rpc_url.as_deref())
}

fn resolve_sp1_network_rpc_url(
    config: &Sp1Config,
    env_rpc_url: Option<&str>,
) -> RaikoResult<Option<String>> {
    let rpc_url = if let Some(rpc_url) = config.rpc_url.clone() {
        rpc_url
    } else {
        let Some(rpc_url) = env_rpc_url.map(str::trim).filter(|value| !value.is_empty()) else {
            return Ok(None);
        };
        rpc_url.to_string()
    };
    if rpc_url != rpc_url.trim() {
        return Err(RaikoError::InvalidRequestConfig(
            "SP1 network RPC URL must not include leading or trailing whitespace".to_string(),
        ));
    }
    Url::parse(&rpc_url).map_err(|err| {
        RaikoError::InvalidRequestConfig(format!("SP1 network RPC URL is invalid: {err}"))
    })?;
    Ok(Some(rpc_url))
}

const fn sp1_network_submission_progress(
    provider_request_id: String,
    config: &Sp1Config,
) -> Sp1NetworkSubmissionProgress {
    Sp1NetworkSubmissionProgress {
        provider_request_id,
        network_mode: config.network_mode,
        fulfillment_strategy: config.fulfillment_strategy,
        skip_simulation: config.skip_simulation,
        cycle_limit: config.cycle_limit,
        timeout_secs: config.timeout_secs,
        max_price_per_pgu: config.max_price_per_pgu,
        auction_timeout_secs: config.auction_timeout_secs,
    }
}

async fn notify_sp1_network_submission(
    observer: Option<&Arc<dyn ProverProgressObserver>>,
    provider_request_id: String,
    config: &Sp1Config,
) {
    if let Some(observer) = observer {
        observer
            .on_progress(&ProverProgress::Sp1NetworkSubmission(
                sp1_network_submission_progress(provider_request_id, config),
            ))
            .await;
    }
}

async fn request_network_proof(
    client: &NetworkProver,
    pk: &SP1ProvingKey,
    stdin: SP1Stdin,
    proof_mode: SP1ProofMode,
    config: &Sp1Config,
    observer: Option<std::sync::Arc<dyn ProverProgressObserver>>,
    stage: &str,
) -> RaikoResult<NetworkProofRequestResult> {
    let timeout = Duration::from_secs(config.timeout_secs);
    let mut stored_request_id = if let Some(observer) = observer.as_ref() {
        observer.load_sp1_network_request_id().await
    } else {
        None
    };
    let mut request_attempt = 1_u64;

    loop {
        let request_id_string = if let Some(request_id_string) = stored_request_id.take() {
            notify_sp1_network_submission(observer.as_ref(), request_id_string.clone(), config)
                .await;

            tracing::info!(
                stage,
                request_id = %request_id_string,
                timeout_secs = config.timeout_secs,
                "Resuming SP1 network proof wait"
            );
            request_id_string
        } else {
            tracing::info!(
                stage,
                proof_mode = ?proof_mode,
                network_mode = %config.network_mode.as_str(),
                fulfillment_strategy = %config.fulfillment_strategy.as_str(),
                skip_simulation = config.skip_simulation,
                cycle_limit = config.cycle_limit,
                timeout_secs = config.timeout_secs,
                max_price_per_pgu = ?config.max_price_per_pgu,
                auction_timeout_secs = ?config.auction_timeout_secs,
                request_attempt,
                "Submitting SP1 network proof request"
            );

            let request_id = client
                .prove(pk, &stdin)
                .mode(proof_mode)
                .strategy(config.fulfillment_strategy.into())
                .skip_simulation(config.skip_simulation)
                .cycle_limit(config.cycle_limit)
                .timeout(timeout)
                .request_async()
                .await
                .map_err(|e| {
                    tracing::error!("Failed to request SP1 {stage} proof: {:?}", e);
                    RaikoError::Guest(format!("SP1 {stage} proof request failed: {e}"))
                })?;

            let request_id_string = request_id.to_string();
            notify_sp1_network_submission(observer.as_ref(), request_id_string.clone(), config)
                .await;

            tracing::info!(
                stage,
                request_id = %request_id_string,
                timeout_secs = config.timeout_secs,
                request_attempt,
                "Waiting for SP1 network proof"
            );
            request_id_string
        };

        let request_id = B256::from_str(&request_id_string).map_err(|e| {
            RaikoError::Guest(format!("Invalid stored SP1 {stage} request id: {e}"))
        })?;
        match wait_sp1_network_proof(client, request_id, timeout, stage, &request_id_string).await?
        {
            Sp1NetworkWaitOutcome::Fulfilled(proof) => {
                return Ok(NetworkProofRequestResult {
                    request_id: request_id_string,
                    proof,
                });
            }
            Sp1NetworkWaitOutcome::RetryRequest(reason) => {
                tracing::warn!(
                    stage,
                    request_id = %request_id_string,
                    request_attempt,
                    reason,
                    "SP1 network proof request reached a retryable terminal state; submitting a new request"
                );
                request_attempt = request_attempt.saturating_add(1);
                tokio::time::sleep(SP1_NETWORK_REQUEST_RETRY_DELAY).await;
            }
        }
    }
}

enum Sp1NetworkWaitOutcome {
    Fulfilled(SP1ProofWithPublicValues),
    RetryRequest(String),
}

async fn wait_sp1_network_proof(
    client: &NetworkProver,
    request_id: B256,
    timeout: Duration,
    stage: &str,
    request_id_string: &str,
) -> RaikoResult<Sp1NetworkWaitOutcome> {
    let mut attempt = 1_u64;
    loop {
        let wait_started = std::time::Instant::now();
        match client.wait_proof(request_id, Some(timeout), None).await {
            Ok(proof) => {
                tracing::info!(
                    stage,
                    request_id = %request_id_string,
                    proof_mode = sp1_proof_mode_name(&proof),
                    attempt,
                    elapsed_ms = wait_started.elapsed().as_millis(),
                    "SP1 network proof received"
                );
                return Ok(Sp1NetworkWaitOutcome::Fulfilled(proof));
            }
            Err(error) => {
                if let Some(network_error) = error.downcast_ref::<Sp1NetworkError>() {
                    match network_error {
                        Sp1NetworkError::RequestUnfulfillable { .. }
                        | Sp1NetworkError::RequestTimedOut { .. }
                        | Sp1NetworkError::RequestAuctionTimedOut { .. } => {
                            return Ok(Sp1NetworkWaitOutcome::RetryRequest(error.to_string()));
                        }
                        Sp1NetworkError::RequestUnexecutable { .. }
                        | Sp1NetworkError::SimulationFailed => {
                            return Err(RaikoError::Guest(format!(
                                "SP1 {stage} network proof failed: {error}"
                            )));
                        }
                        Sp1NetworkError::RpcError(_) | Sp1NetworkError::Other(_) => {}
                    }
                }
                tracing::warn!(
                    stage,
                    request_id = %request_id_string,
                    attempt,
                    error = ?error,
                    "SP1 network proof wait failed; retrying existing request id"
                );
                attempt = attempt.saturating_add(1);
                tokio::time::sleep(SP1_NETWORK_WAIT_RETRY_DELAY).await;
            }
        }
    }
}

fn insert_sp1_metadata(
    extra_data: Option<serde_json::Value>,
    metadata: serde_json::Value,
) -> RaikoResult<Option<serde_json::Value>> {
    let mut extra_data = extra_data.unwrap_or_else(|| serde_json::json!({}));
    let Some(root) = extra_data.as_object_mut() else {
        return Err(RaikoError::Guest(
            "SP1 extra_data root must be a JSON object".to_string(),
        ));
    };
    root.insert("sp1".to_string(), metadata);
    Ok(Some(extra_data))
}

#[cfg(test)]
mod tests {
    use super::{
        encode_sp1_onchain_payload, load_sp1_subproof_for_aggregation,
        remote_verifier_program_vkey, remote_verifier_proof_bytes, resolve_sp1_network_rpc_url,
        sp1_vk_uuid,
    };
    use alloy_primitives::B256;
    use raiko2_guests::{Sp1ShastaGuestElves, load_sp1_shasta_guest_elves};
    use raiko2_pipeline::ProofStage;
    use raiko2_pipeline::forks::shasta::sp1_shasta_backend_from_elves;
    use raiko2_primitives::Proof;
    use raiko2_primitives_shasta::instance::{sp1_contract_block_program_id, words_to_bytes_le};
    use sp1_sdk::{
        HashableKey, Prover as _, ProverClient, SP1ProofMode, SP1ProofWithPublicValues,
        SP1VerifyingKey,
    };
    use std::str::FromStr;

    fn sp1_test_elves() -> Sp1ShastaGuestElves {
        load_sp1_shasta_guest_elves().expect("load SP1 Shasta guest ELFs")
    }

    fn legacy_raiko_sp1_aggregation_payload(
        aggregation_vk: &SP1VerifyingKey,
        block_vk: &SP1VerifyingKey,
        proof_bytes: impl AsRef<[u8]>,
    ) -> String {
        format!(
            "{}{}{}",
            aggregation_vk.bytes32(),
            alloy_primitives::hex::encode(block_vk.hash_bytes()),
            alloy_primitives::hex::encode(proof_bytes)
        )
    }

    #[test]
    fn sp1_setup_cache_requires_preload_before_use() {
        let prover = super::Sp1Prover::new(super::Sp1Config::default());

        let Err(err) = prover.cached_setup_for_stage(ProofStage::Proposal) else {
            panic!("setup should not be initialized");
        };

        assert!(err.to_string().contains("setup is not initialized"));
    }

    #[test]
    fn sp1_new_with_backend_preloads_setup_cache() {
        let backend = sp1_shasta_backend_from_elves(sp1_test_elves());
        let prover = super::Sp1Prover::new_with_backend(super::Sp1Config::default(), &backend)
            .expect("preload SP1 setup");

        assert!(prover.cached_setup_for_stage(ProofStage::Proposal).is_ok());
        assert!(
            prover
                .cached_setup_for_stage(ProofStage::Aggregation)
                .is_ok()
        );
    }

    #[test]
    fn sp1_network_rpc_url_trims_and_validates_env_override() {
        let config = super::Sp1Config::default();

        let rpc_url = resolve_sp1_network_rpc_url(&config, Some(" https://example.invalid/rpc "))
            .expect("valid env rpc url");

        assert_eq!(rpc_url.as_deref(), Some("https://example.invalid/rpc"));
    }

    #[test]
    fn sp1_network_rpc_url_uses_sdk_default_without_override() {
        let config = super::Sp1Config::default();

        let rpc_url = resolve_sp1_network_rpc_url(&config, None).expect("optional rpc url");

        assert_eq!(rpc_url, None);
    }

    #[test]
    fn sp1_network_rpc_url_prefers_config_over_env() {
        let config = super::Sp1Config {
            rpc_url: Some("https://config.example.invalid/rpc".to_string()),
            ..super::Sp1Config::default()
        };

        let rpc_url = resolve_sp1_network_rpc_url(&config, Some("https://env.example.invalid/rpc"))
            .expect("valid config rpc url");

        assert_eq!(
            rpc_url.as_deref(),
            Some("https://config.example.invalid/rpc")
        );
    }

    #[test]
    fn sp1_network_rpc_url_rejects_untrimmed_config_override() {
        let config = super::Sp1Config {
            rpc_url: Some(" https://config.example.invalid/rpc ".to_string()),
            ..super::Sp1Config::default()
        };

        let err = resolve_sp1_network_rpc_url(&config, Some("https://env.example.invalid/rpc"))
            .expect_err("untrimmed config rpc url");

        assert!(
            err.to_string()
                .contains("SP1 network RPC URL must not include leading or trailing whitespace")
        );
    }

    #[test]
    fn sp1_network_rpc_url_rejects_invalid_env_override() {
        let config = super::Sp1Config::default();

        let err = resolve_sp1_network_rpc_url(&config, Some("not a url"))
            .expect_err("invalid env rpc url");

        assert!(err.to_string().contains("SP1 network RPC URL is invalid"));
    }

    #[test]
    fn remote_verifier_program_vkey_matches_contract_bytes32_encoding() {
        let client = ProverClient::builder().mock().build();
        let elves = sp1_test_elves();
        let (_, vk) = client.setup(elves.proposal.as_ref());

        let expected = B256::from_str(&vk.bytes32()).expect("valid bytes32 encoding");
        let old_buggy = B256::from_slice(&vk.hash_bytes());

        assert_eq!(remote_verifier_program_vkey(&vk), expected);
        assert_ne!(remote_verifier_program_vkey(&vk), old_buggy);
    }

    #[test]
    fn raw_image_id_hex_roundtrips_through_guest_word_encoding() {
        let expected = B256::from([0x5a; 32]);
        let words = super::sp1_image_id_words_from_uuid(&expected.to_string()).expect("image id");

        assert_eq!(B256::from(words_to_bytes_le(&words)), expected);
    }

    #[test]
    fn sp1_vkey_words_match_contract_block_program_encoding() {
        let client = ProverClient::builder().mock().build();
        let elves = sp1_test_elves();
        let (_, vk) = client.setup(elves.proposal.as_ref());
        let words = super::sp1_image_id_words_from_uuid(&sp1_vk_uuid(&vk)).expect("image id");

        assert_eq!(words, vk.hash_u32());
        assert_eq!(
            sp1_contract_block_program_id(&words),
            B256::from_slice(&vk.hash_bytes())
        );
        assert_ne!(
            B256::from(words_to_bytes_le(&words)),
            B256::from_slice(&vk.hash_bytes())
        );
    }

    #[test]
    fn sp1_aggregation_public_input_matches_contract_payload_program_id() {
        let client = ProverClient::builder().mock().build();
        let elves = sp1_test_elves();
        let (_, block_vk) = client.setup(elves.proposal.as_ref());
        let (_, aggregation_vk) = client.setup(elves.aggregation.as_ref());
        let words = super::sp1_image_id_words_from_uuid(&sp1_vk_uuid(&block_vk)).expect("image id");
        let proof_bytes = [0xaa, 0xbb, 0xcc];

        let payload = legacy_raiko_sp1_aggregation_payload(&aggregation_vk, &block_vk, proof_bytes);
        let payload_bytes = alloy_primitives::hex::decode(payload.strip_prefix("0x").unwrap())
            .expect("payload hex");
        let block_program_from_contract_payload = B256::from_slice(&payload_bytes[32..64]);

        assert_eq!(
            block_program_from_contract_payload,
            sp1_contract_block_program_id(&words)
        );
        assert_ne!(
            block_program_from_contract_payload,
            B256::from(words_to_bytes_le(&words))
        );
    }

    #[test]
    fn remote_verifier_proof_bytes_matches_legacy_raiko_semantics() {
        let client = ProverClient::builder().mock().build();
        let elves = sp1_test_elves();
        let (pk, _) = client.setup(elves.proposal.as_ref());

        let core = SP1ProofWithPublicValues::create_mock_proof(
            &pk,
            sp1_sdk::SP1PublicValues::new(),
            SP1ProofMode::Core,
            sp1_sdk::SP1_CIRCUIT_VERSION,
        );
        let compressed = SP1ProofWithPublicValues::create_mock_proof(
            &pk,
            sp1_sdk::SP1PublicValues::new(),
            SP1ProofMode::Compressed,
            sp1_sdk::SP1_CIRCUIT_VERSION,
        );
        let plonk = SP1ProofWithPublicValues::create_mock_proof(
            &pk,
            sp1_sdk::SP1PublicValues::new(),
            SP1ProofMode::Plonk,
            sp1_sdk::SP1_CIRCUIT_VERSION,
        );

        assert!(remote_verifier_proof_bytes(&core).is_none());
        assert!(remote_verifier_proof_bytes(&compressed).is_none());
        assert!(remote_verifier_proof_bytes(&plonk).is_none());
    }

    #[test]
    fn sp1_aggregation_payload_layout_matches_legacy_raiko() {
        let client = ProverClient::builder().mock().build();
        let elves = sp1_test_elves();
        let (_, block_vk) = client.setup(elves.proposal.as_ref());
        let (_, aggregation_vk) = client.setup(elves.aggregation.as_ref());
        let proof_bytes = [0xaa, 0xbb, 0xcc];

        let payload = encode_sp1_onchain_payload(
            &[
                aggregation_vk.bytes32(),
                alloy_primitives::hex::encode_prefixed(block_vk.hash_bytes()),
            ],
            &proof_bytes,
        );

        assert_eq!(
            payload,
            legacy_raiko_sp1_aggregation_payload(&aggregation_vk, &block_vk, proof_bytes)
        );
    }

    #[test]
    fn load_sp1_subproof_for_aggregation_prefers_quote() {
        let client = ProverClient::builder().mock().build();
        let elves = sp1_test_elves();
        let (pk, _) = client.setup(elves.proposal.as_ref());
        let compressed = SP1ProofWithPublicValues::create_mock_proof(
            &pk,
            sp1_sdk::SP1PublicValues::new(),
            SP1ProofMode::Compressed,
            sp1_sdk::SP1_CIRCUIT_VERSION,
        );
        let proof = Proof {
            quote: Some(serde_json::to_string(&compressed.proof).expect("serialize quote")),
            proof: Some("0xdeadbeef".to_string()),
            ..Proof::default()
        };

        let loaded = load_sp1_subproof_for_aggregation(&proof).expect("load subproof");
        assert!(matches!(loaded, sp1_sdk::SP1Proof::Compressed(_)));
    }

    #[test]
    fn load_sp1_subproof_for_aggregation_accepts_legacy_bincode_payload() {
        let client = ProverClient::builder().mock().build();
        let elves = sp1_test_elves();
        let (pk, _) = client.setup(elves.proposal.as_ref());
        let compressed = SP1ProofWithPublicValues::create_mock_proof(
            &pk,
            sp1_sdk::SP1PublicValues::new(),
            SP1ProofMode::Compressed,
            sp1_sdk::SP1_CIRCUIT_VERSION,
        );
        let encoded = bincode::serialize(&compressed).expect("serialize legacy payload");
        let proof = Proof {
            proof: Some(alloy_primitives::hex::encode_prefixed(encoded)),
            ..Proof::default()
        };

        let loaded = load_sp1_subproof_for_aggregation(&proof).expect("load legacy subproof");
        assert!(matches!(loaded, sp1_sdk::SP1Proof::Compressed(_)));
    }
}
