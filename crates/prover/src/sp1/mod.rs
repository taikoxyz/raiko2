//! SP1 zkVM Prover for Raiko V2
//!
//! This module provides the SP1 prover implementation for generating
//! zero-knowledge proofs of Taiko block execution.

mod types;

pub use types::{
    ExecutionMode, ProverMode, RecursionMode, Sp1Config, Sp1ConfigError, Sp1ConfigOverrides,
    Sp1ExecutionMetadata, Sp1FulfillmentStrategy, Sp1NetworkMetadata, Sp1NetworkMode,
    Sp1NetworkSubmissionProgress, Sp1RequestContext, Sp1Response,
};

use alloy_primitives::Bytes;
use raiko2_pipeline::{ProofStage, ProverBackend};
use raiko2_primitives::{AggregationGuestInput, Proof, ProverConfig, RaikoError, RaikoResult};
use raiko2_primitives_shasta::GuestInput;
use serde::Deserialize;
use sp1_sdk::{
    Elf, HashableKey, NetworkProver, ProveRequest as _, Prover as _, ProverClient, ProvingKey as _,
    SP1Proof, SP1ProofMode, SP1ProofWithPublicValues, SP1ProvingKey, SP1Stdin,
};
use std::time::Duration;
use tracing::info;

use crate::{
    GuestInputCodec, ProverProgress, ProverProgressObserver, build_shasta_aggregation_input,
    parse_shasta_aggregation_input_hash, parse_shasta_proposal_input_hash, with_shasta_extra_data,
};

/// SP1 Prover for Shasta proposal proofs.
pub struct Sp1Prover {
    config: Sp1Config,
}

impl Sp1Prover {
    /// Create a new SP1 prover with the given configuration.
    #[must_use]
    pub const fn new(config: Sp1Config) -> Self {
        Self { config }
    }

    fn resolve_config_for_request(
        &self,
        config: &ProverConfig,
        context: Sp1RequestContext,
    ) -> RaikoResult<Sp1Config> {
        let overrides = match config.get("sp1") {
            Some(value) => Sp1ConfigOverrides::deserialize(value).map_err(|e| {
                RaikoError::InvalidRequestConfig(format!("Failed to parse 'sp1' prover args: {e}"))
            })?,
            None => Sp1ConfigOverrides::default(),
        };
        self.config
            .resolve_request_config(Some(&overrides), context)
            .map_err(|err| RaikoError::InvalidRequestConfig(err.to_string()))
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

        // Prepare stdin for SP1 guest program
        // The guest reads a single GuestInput via typed IO.
        let mut stdin = SP1Stdin::new();
        let guest_input: GuestInput = bincode::deserialize(input.as_ref())
            .map_err(|e| RaikoError::Guest(format!("Failed to deserialize input: {e}")))?;
        stdin.write(&guest_input);

        // Use the proposal ELF selected by the configured backend.
        let elf = Elf::from(backend.elf(ProofStage::Proposal)?);
        match effective_config.mode {
            ExecutionMode::Execute => {
                if effective_config.prover == ProverMode::Network {
                    return Err(RaikoError::InvalidRequestConfig(
                        "sp1.mode=execute does not support sp1.prover=network".to_string(),
                    ));
                }
                let client = ProverClient::builder().light().build().await;
                let (public_values, execution_report) =
                    client.execute(elf, stdin).await.map_err(|e| {
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
            }
            ExecutionMode::Prove => {
                let proof_mode: SP1ProofMode = effective_config.recursion.into();
                match effective_config.prover {
                    ProverMode::Mock => {
                        let client = ProverClient::builder().mock().build().await;
                        prove_proposal_with_client(
                            &client,
                            elf,
                            stdin,
                            proof_mode,
                            effective_config.verify,
                            &guest_input,
                        )
                        .await
                    }
                    ProverMode::Local => {
                        let client = ProverClient::builder().cpu().build().await;
                        prove_proposal_with_client(
                            &client,
                            elf,
                            stdin,
                            proof_mode,
                            effective_config.verify,
                            &guest_input,
                        )
                        .await
                    }
                    ProverMode::Network => {
                        let client = build_network_prover(&effective_config).await?;
                        prove_proposal_with_network_client(
                            &client,
                            elf,
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
        backend: &B,
    ) -> RaikoResult<Proof> {
        let effective_config =
            self.resolve_config_for_request(config, Sp1RequestContext::Aggregation)?;
        info!(
            "Starting SP1 aggregation proof generation with {} proofs...",
            input.proofs.len()
        );

        let aggregation_input = build_shasta_aggregation_input(&input.proofs)?;

        // Prepare stdin for SP1 aggregation guest program
        // The guest reads ShastaZkAggregationGuestInput via sp1_zkvm::io::read()
        let mut stdin = SP1Stdin::new();
        stdin.write(&aggregation_input);

        // Get the proposal prover's verifying key for proof verification.
        // The proposal proofs were generated with the proposal ELF.
        let elf = Elf::from(backend.elf(ProofStage::Aggregation)?);
        let proof_mode: SP1ProofMode = effective_config.recursion.into();

        match effective_config.prover {
            ProverMode::Mock => {
                let client = ProverClient::builder().mock().build().await;
                aggregate_with_client(
                    &client,
                    elf,
                    backend,
                    &input,
                    stdin,
                    proof_mode,
                    effective_config.verify,
                )
                .await
            }
            ProverMode::Local => {
                let client = ProverClient::builder().cpu().build().await;
                aggregate_with_client(
                    &client,
                    elf,
                    backend,
                    &input,
                    stdin,
                    proof_mode,
                    effective_config.verify,
                )
                .await
            }
            ProverMode::Network => {
                let client = build_network_prover(&effective_config).await?;
                aggregate_with_network_client(
                    &client,
                    elf,
                    backend,
                    &input,
                    stdin,
                    proof_mode,
                    &effective_config,
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

async fn prove_proposal_with_client<C>(
    client: &C,
    elf: Elf,
    stdin: SP1Stdin,
    proof_mode: SP1ProofMode,
    verify: bool,
    guest_input: &GuestInput,
) -> RaikoResult<Proof>
where
    C: sp1_sdk::Prover,
{
    let pk = client.setup(elf).await.map_err(|e| {
        tracing::error!("Failed to setup SP1 proposal proving key: {:?}", e);
        RaikoError::Guest(format!("SP1 proposal setup failed: {e}"))
    })?;
    let proof = client
        .prove(&pk, stdin)
        .mode(proof_mode)
        .await
        .map_err(|e| {
            tracing::error!("Failed to generate SP1 proposal proof: {:?}", e);
            RaikoError::Guest(format!("SP1 proposal proof generation failed: {e}"))
        })?;

    if verify {
        client
            .verify(&proof, pk.verifying_key(), None)
            .map_err(|e| {
                tracing::error!("Failed to verify SP1 proposal proof: {:?}", e);
                RaikoError::Guest(format!("SP1 proposal proof verification failed: {e}"))
            })?;
    }

    let public_values = proof.public_values.as_slice();
    let input_hash = parse_shasta_proposal_input_hash(public_values)?;
    let proof_bytes = bincode::serialize(&proof)
        .map_err(|e| RaikoError::Guest(format!("Failed to serialize proof: {e}")))?;

    Ok(Sp1Response {
        proof: Some(alloy_primitives::hex::encode_prefixed(&proof_bytes)),
        vkey_hash: Some(pk.verifying_key().bytes32().clone()),
        input: input_hash,
        sp1_proof: Some(proof),
        vkey: Some(pk.verifying_key().clone()),
        extra_data: with_shasta_extra_data(&guest_input.proof_carry_data, "sp1", None)?,
    }
    .into())
}

async fn prove_proposal_with_network_client(
    client: &NetworkProver,
    elf: Elf,
    stdin: SP1Stdin,
    proof_mode: SP1ProofMode,
    config: &Sp1Config,
    guest_input: &GuestInput,
    observer: Option<std::sync::Arc<dyn ProverProgressObserver>>,
) -> RaikoResult<Proof> {
    let pk = client.setup(elf).await.map_err(|e| {
        tracing::error!("Failed to setup SP1 network proposal proving key: {:?}", e);
        RaikoError::Guest(format!("SP1 proposal setup failed: {e}"))
    })?;
    let request =
        request_network_proof(client, &pk, stdin, proof_mode, config, observer, "proposal").await?;

    if config.verify {
        client
            .verify(&request.proof, pk.verifying_key(), None)
            .map_err(|e| {
                tracing::error!("Failed to verify SP1 proposal proof: {:?}", e);
                RaikoError::Guest(format!("SP1 proposal proof verification failed: {e}"))
            })?;
    }

    let public_values = request.proof.public_values.as_slice();
    let input_hash = parse_shasta_proposal_input_hash(public_values)?;
    let proof_bytes = bincode::serialize(&request.proof)
        .map_err(|e| RaikoError::Guest(format!("Failed to serialize proof: {e}")))?;
    let base_extra_data = with_shasta_extra_data(&guest_input.proof_carry_data, "sp1", None)?;
    let network_metadata =
        serde_json::to_value(Sp1NetworkMetadata::from_config(request.request_id, config))
            .map_err(|e| RaikoError::Guest(format!("Failed to serialize SP1 metadata: {e}")))?;

    Ok(Sp1Response {
        proof: Some(alloy_primitives::hex::encode_prefixed(&proof_bytes)),
        vkey_hash: Some(pk.verifying_key().bytes32().clone()),
        input: input_hash,
        sp1_proof: Some(request.proof),
        vkey: Some(pk.verifying_key().clone()),
        extra_data: insert_sp1_metadata(
            base_extra_data,
            serde_json::json!({ "network": network_metadata }),
        )?,
    }
    .into())
}

async fn aggregate_with_client<C, B>(
    client: &C,
    elf: Elf,
    backend: &B,
    input: &AggregationGuestInput,
    mut stdin: SP1Stdin,
    proof_mode: SP1ProofMode,
    verify: bool,
) -> RaikoResult<Proof>
where
    C: sp1_sdk::Prover,
    B: ProverBackend,
{
    let proposal_elf = Elf::from(backend.elf(ProofStage::Proposal)?);
    let proposal_pk = client.setup(proposal_elf).await.map_err(|e| {
        tracing::error!(
            "Failed to setup SP1 proposal verifying key for aggregation: {:?}",
            e
        );
        RaikoError::Guest(format!("SP1 aggregation proposal setup failed: {e}"))
    })?;
    let verifier_key = proposal_pk.verifying_key().clone();

    for proof in &input.proofs {
        if let Some(proof_hex) = &proof.proof {
            let proof_bytes = alloy_primitives::hex::decode(proof_hex)
                .map_err(|e| RaikoError::Guest(format!("Failed to decode proof hex: {e}")))?;
            let sp1_proof_with_pv: SP1ProofWithPublicValues = bincode::deserialize(&proof_bytes)
                .map_err(|e| {
                    RaikoError::Guest(format!(
                        "Failed to deserialize SP1ProofWithPublicValues: {e}"
                    ))
                })?;
            match sp1_proof_with_pv.proof {
                SP1Proof::Compressed(reduce_proof) => {
                    stdin.write_proof(*reduce_proof, verifier_key.vk.clone());
                }
                _ => {
                    return Err(RaikoError::Guest(
                        "Aggregation requires Compressed proofs".to_string(),
                    ));
                }
            }
        }
    }

    let pk = client.setup(elf).await.map_err(|e| {
        tracing::error!("Failed to setup SP1 aggregation proving key: {:?}", e);
        RaikoError::Guest(format!("SP1 aggregation setup failed: {e}"))
    })?;
    let proof = client
        .prove(&pk, stdin)
        .mode(proof_mode)
        .await
        .map_err(|e| {
            tracing::error!("Failed to generate SP1 aggregation proof: {:?}", e);
            RaikoError::Guest(format!("SP1 aggregation proof generation failed: {e}"))
        })?;

    if verify {
        client
            .verify(&proof, pk.verifying_key(), None)
            .map_err(|e| {
                tracing::error!("Failed to verify SP1 aggregation proof: {:?}", e);
                RaikoError::Guest(format!("SP1 aggregation proof verification failed: {e}"))
            })?;
    }

    let public_values = proof.public_values.as_slice();
    let agg_input_hash = parse_shasta_aggregation_input_hash(public_values);
    let proof_bytes = bincode::serialize(&proof)
        .map_err(|e| RaikoError::Guest(format!("Failed to serialize proof: {e}")))?;

    Ok(Sp1Response {
        proof: Some(alloy_primitives::hex::encode_prefixed(&proof_bytes)),
        vkey_hash: Some(pk.verifying_key().bytes32().clone()),
        input: agg_input_hash,
        sp1_proof: Some(proof),
        vkey: Some(pk.verifying_key().clone()),
        extra_data: None,
    }
    .into())
}

async fn aggregate_with_network_client<B>(
    client: &NetworkProver,
    elf: Elf,
    backend: &B,
    input: &AggregationGuestInput,
    mut stdin: SP1Stdin,
    proof_mode: SP1ProofMode,
    config: &Sp1Config,
) -> RaikoResult<Proof>
where
    B: ProverBackend,
{
    let proposal_elf = Elf::from(backend.elf(ProofStage::Proposal)?);
    let proposal_pk = client.setup(proposal_elf).await.map_err(|e| {
        tracing::error!(
            "Failed to setup SP1 proposal verifying key for aggregation: {:?}",
            e
        );
        RaikoError::Guest(format!("SP1 aggregation proposal setup failed: {e}"))
    })?;
    let verifier_key = proposal_pk.verifying_key().clone();

    for proof in &input.proofs {
        if let Some(proof_hex) = &proof.proof {
            let proof_bytes = alloy_primitives::hex::decode(proof_hex)
                .map_err(|e| RaikoError::Guest(format!("Failed to decode proof hex: {e}")))?;
            let sp1_proof_with_pv: SP1ProofWithPublicValues = bincode::deserialize(&proof_bytes)
                .map_err(|e| {
                    RaikoError::Guest(format!(
                        "Failed to deserialize SP1ProofWithPublicValues: {e}"
                    ))
                })?;
            match sp1_proof_with_pv.proof {
                SP1Proof::Compressed(reduce_proof) => {
                    stdin.write_proof(*reduce_proof, verifier_key.vk.clone());
                }
                _ => {
                    return Err(RaikoError::Guest(
                        "Aggregation requires Compressed proofs".to_string(),
                    ));
                }
            }
        }
    }

    let pk = client.setup(elf).await.map_err(|e| {
        tracing::error!("Failed to setup SP1 aggregation proving key: {:?}", e);
        RaikoError::Guest(format!("SP1 aggregation setup failed: {e}"))
    })?;
    let request =
        request_network_proof(client, &pk, stdin, proof_mode, config, None, "aggregation").await?;

    if config.verify {
        client
            .verify(&request.proof, pk.verifying_key(), None)
            .map_err(|e| {
                tracing::error!("Failed to verify SP1 aggregation proof: {:?}", e);
                RaikoError::Guest(format!("SP1 aggregation proof verification failed: {e}"))
            })?;
    }

    let public_values = request.proof.public_values.as_slice();
    let agg_input_hash = parse_shasta_aggregation_input_hash(public_values);
    let proof_bytes = bincode::serialize(&request.proof)
        .map_err(|e| RaikoError::Guest(format!("Failed to serialize proof: {e}")))?;
    let network_metadata =
        serde_json::to_value(Sp1NetworkMetadata::from_config(request.request_id, config))
            .map_err(|e| RaikoError::Guest(format!("Failed to serialize SP1 metadata: {e}")))?;

    Ok(Sp1Response {
        proof: Some(alloy_primitives::hex::encode_prefixed(&proof_bytes)),
        vkey_hash: Some(pk.verifying_key().bytes32().clone()),
        input: agg_input_hash,
        sp1_proof: Some(request.proof),
        vkey: Some(pk.verifying_key().clone()),
        extra_data: insert_sp1_metadata(None, serde_json::json!({ "network": network_metadata }))?,
    }
    .into())
}

async fn build_network_prover(config: &Sp1Config) -> RaikoResult<NetworkProver> {
    let private_key = std::env::var("NETWORK_PRIVATE_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            RaikoError::InvalidRequestConfig(
                "NETWORK_PRIVATE_KEY must be set for sp1.prover=network".to_string(),
            )
        })?;
    let builder = ProverClient::builder()
        .network_for(config.network_mode.into())
        .private_key(&private_key);
    let builder = if let Some(rpc_url) = config.rpc_url.as_deref() {
        builder.rpc_url(rpc_url)
    } else {
        builder
    };
    Ok(builder.build().await)
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
    let request_id = client
        .prove(pk, stdin)
        .mode(proof_mode)
        .strategy(config.fulfillment_strategy.into())
        .skip_simulation(config.skip_simulation)
        .cycle_limit(config.cycle_limit)
        .timeout(timeout)
        .request()
        .await
        .map_err(|e| {
            tracing::error!("Failed to request SP1 {stage} proof: {:?}", e);
            RaikoError::Guest(format!("SP1 {stage} proof request failed: {e}"))
        })?;

    let request_id_string = request_id.to_string();
    if let Some(observer) = observer {
        observer
            .on_progress(&ProverProgress::Sp1NetworkSubmission(
                Sp1NetworkSubmissionProgress {
                    provider_request_id: request_id_string.clone(),
                    network_mode: config.network_mode,
                    fulfillment_strategy: config.fulfillment_strategy,
                    skip_simulation: config.skip_simulation,
                    cycle_limit: config.cycle_limit,
                    timeout_secs: config.timeout_secs,
                },
            ))
            .await;
    }

    let proof = client
        .wait_proof(request_id, Some(timeout), None)
        .await
        .map_err(|e| {
            tracing::error!("Failed to wait for SP1 {stage} proof: {:?}", e);
            RaikoError::Guest(format!("SP1 {stage} network proof failed: {e}"))
        })?;

    Ok(NetworkProofRequestResult {
        request_id: request_id_string,
        proof,
    })
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
