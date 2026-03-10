//! RISC0 zkVM Prover for Raiko V2
//!
//! This module provides the RISC0 prover implementation for generating
//! zero-knowledge proofs of Taiko block execution.

mod types;

pub use types::{Risc0Config, Risc0ExecutionMetadata, Risc0Response};

use alloy_primitives::{B256, Bytes};
use raiko2_pipeline::{ProofStage, ProverBackend};
use raiko2_primitives::{AggregationGuestInput, Proof, ProverConfig, RaikoError, RaikoResult};
use raiko2_primitives_shasta::{GuestInput, build_proof_carry_data};
use risc0_zkvm::{
    Digest, ExecutorEnv, FakeReceipt, ProverOpts, Receipt, VerifierContext, compute_image_id,
    default_executor, default_prover,
};
use tracing::info;

use crate::{
    GuestInputCodec, parse_proof_carry_data, parse_shasta_aggregation_input,
    validate_shasta_aggregation_lengths,
};

/// RISC0 Prover for Shasta proposal proofs.
pub struct Risc0Prover {
    config: Risc0Config,
}

impl Risc0Prover {
    /// Create a new RISC0 prover with the given configuration.
    #[must_use]
    pub const fn new(config: Risc0Config) -> Self {
        Self { config }
    }

    fn prover_opts(&self) -> ProverOpts {
        if self.config.mock {
            ProverOpts::default().with_dev_mode(true)
        } else if self.config.snark {
            ProverOpts::groth16()
        } else {
            ProverOpts::default()
        }
    }

    fn verify_receipt(&self, receipt: &Receipt, image_id: Digest) -> RaikoResult<()> {
        if !self.config.verify {
            return Ok(());
        }

        if self.config.mock {
            receipt.verify_with_context(&VerifierContext::default().with_dev_mode(true), image_id)
        } else {
            receipt.verify(image_id)
        }
        .map_err(|e| {
            tracing::error!("Failed to verify RISC0 receipt: {:?}", e);
            RaikoError::Guest(format!("RISC0 receipt verification failed: {e}"))
        })
    }

    fn mock_extra_data(
        session: &risc0_zkvm::SessionInfo,
        image_id: Digest,
        input_hash: B256,
        journal_bytes: usize,
        mode: &str,
    ) -> RaikoResult<Option<serde_json::Value>> {
        let metadata = Risc0ExecutionMetadata::from_session(
            session,
            alloy_primitives::hex::encode_prefixed(image_id.as_bytes()),
            input_hash,
            journal_bytes,
            mode,
            true,
        );
        serde_json::to_value(metadata)
            .map(Some)
            .map_err(|e| RaikoError::Guest(format!("Failed to serialize RISC0 metadata: {e}")))
    }
}

impl GuestInputCodec<GuestInput> for Risc0Prover {
    fn encode(&self, input: &GuestInput, _config: &ProverConfig) -> RaikoResult<Bytes> {
        let bytes = bincode::serialize(input)
            .map_err(|e| RaikoError::Guest(format!("Failed to serialize input: {e}")))?;
        Ok(Bytes::from(bytes))
    }
}

#[async_trait::async_trait]
impl<B> crate::Prover<B> for Risc0Prover
where
    B: ProverBackend,
{
    type GuestInput = GuestInput;

    fn encode(&self, input: &Self::GuestInput, config: &ProverConfig) -> RaikoResult<Bytes> {
        GuestInputCodec::encode(self, input, config)
    }

    fn prepare_config_for_input(
        &self,
        input: &Self::GuestInput,
        config: &mut ProverConfig,
    ) -> RaikoResult<()> {
        config["proof_carry_data"] = serde_json::to_value(build_proof_carry_data(input))?;
        Ok(())
    }

    async fn prove_encoded(
        &self,
        input: Bytes,
        config: &ProverConfig,
        backend: &B,
    ) -> RaikoResult<Proof> {
        let input: GuestInput = bincode::deserialize(input.as_ref())
            .map_err(|e| RaikoError::Guest(format!("Failed to deserialize input: {e}")))?;
        info!("Starting RISC0 proposal proof generation...");

        // Extract ProofCarryData from config if available
        let proof_carry_data = parse_proof_carry_data(config);
        let elf = backend.elf(ProofStage::Proposal)?.to_vec();
        let prover_config = self.config.clone();
        let opts = self.prover_opts();

        tokio::task::spawn_blocking(move || {
            // Build the executor environment with both inputs
            // The guest reads:
            // 1. GuestInput via env::read()
            // 2. ProofCarryData via env::read()
            let env = ExecutorEnv::builder()
                .write(&input)
                .map_err(|e| RaikoError::Guest(format!("Failed to write input: {e}")))?
                .write(&proof_carry_data)
                .map_err(|e| RaikoError::Guest(format!("Failed to write proof_carry_data: {e}")))?
                .build()
                .map_err(|e| RaikoError::Guest(format!("Failed to build env: {e}")))?;

            let image_id = compute_image_id(&elf)
                .map_err(|e| RaikoError::Guest(format!("Failed to compute image id: {e}")))?;

            let (receipt, extra_data) = if prover_config.mock {
                let session = default_executor().execute(env, &elf).map_err(|e| {
                    tracing::error!("Failed to execute RISC0 proposal in mock mode: {:?}", e);
                    RaikoError::Guest(format!("RISC0 proposal mock execution failed: {e}"))
                })?;
                let claim = session.receipt_claim.clone().ok_or_else(|| {
                    RaikoError::Guest(
                        "RISC0 proposal mock execution returned no receipt claim".to_string(),
                    )
                })?;
                let receipt = Receipt::try_from(FakeReceipt::new(claim)).map_err(|e| {
                    RaikoError::Guest(format!(
                        "Failed to convert RISC0 proposal mock receipt: {e}"
                    ))
                })?;

                let journal_bytes = &receipt.journal.bytes;
                let input_hash = if journal_bytes.len() >= 32 {
                    B256::from_slice(&journal_bytes[..32])
                } else {
                    B256::default()
                };
                let extra_data = Self::mock_extra_data(
                    &session,
                    image_id,
                    input_hash,
                    journal_bytes.len(),
                    "mock",
                )?;
                (receipt, extra_data)
            } else {
                let receipt = default_prover()
                    .prove_with_opts(env, &elf, &opts)
                    .map_err(|e| {
                        tracing::error!("Failed to generate RISC0 proposal proof: {:?}", e);
                        RaikoError::Guest(format!("RISC0 proposal proof generation failed: {e}"))
                    })?
                    .receipt;
                (receipt, None)
            };

            info!("RISC0 proposal proof generated successfully");
            if prover_config.mock {
                info!("RISC0 mock mode enabled; proposal receipt is fake but journal is real");
            }
            Risc0Prover {
                config: prover_config.clone(),
            }
            .verify_receipt(&receipt, image_id)?;
            if prover_config.verify {
                info!("RISC0 proposal proof verified successfully");
            }

            let journal_bytes = &receipt.journal.bytes;
            let input_hash = if journal_bytes.len() >= 32 {
                B256::from_slice(&journal_bytes[..32])
            } else {
                B256::default()
            };

            info!(
                "Generated proposal receipt journal: {:?}",
                alloy_primitives::hex::encode_prefixed(journal_bytes.clone())
            );

            let receipt_json = serde_json::to_string(&receipt).unwrap_or_default();

            Ok::<Proof, RaikoError>(
                Risc0Response {
                    proof: alloy_primitives::hex::encode_prefixed(journal_bytes),
                    receipt: receipt_json,
                    image_id: alloy_primitives::hex::encode_prefixed(image_id.as_bytes()),
                    input: input_hash,
                    extra_data,
                }
                .into(),
            )
        })
        .await
        .map_err(|e| RaikoError::Guest(format!("RISC0 proposal proof task join failed: {e}")))?
    }

    async fn aggregate(
        &self,
        input: AggregationGuestInput,
        config: &ProverConfig,
        backend: &B,
    ) -> RaikoResult<Proof> {
        info!(
            "Starting RISC0 aggregation proof generation with {} proofs...",
            input.proofs.len()
        );

        // Extract ShastaZkAggregationGuestInput from config
        let aggregation_input = parse_shasta_aggregation_input(config)?;
        validate_shasta_aggregation_lengths(&aggregation_input)?;
        let elf = backend.elf(ProofStage::Aggregation)?.to_vec();
        let prover_config = self.config.clone();
        let opts = self.prover_opts();

        tokio::task::spawn_blocking(move || {
            let mut env_builder = ExecutorEnv::builder();
            env_builder.write(&aggregation_input).map_err(|e| {
                RaikoError::Guest(format!("Failed to write aggregation input: {e}"))
            })?;

            for proof in &input.proofs {
                if let Some(receipt_json) = &proof.quote {
                    let receipt: Receipt = serde_json::from_str(receipt_json).map_err(|e| {
                        RaikoError::Guest(format!("Failed to parse RISC0 receipt: {e}"))
                    })?;
                    env_builder.add_assumption(receipt);
                }
            }

            let env = env_builder
                .build()
                .map_err(|e| RaikoError::Guest(format!("Failed to build env: {e}")))?;

            let image_id = compute_image_id(&elf)
                .map_err(|e| RaikoError::Guest(format!("Failed to compute image id: {e}")))?;

            let (receipt, extra_data) = if prover_config.mock {
                let session = default_executor().execute(env, &elf).map_err(|e| {
                    tracing::error!("Failed to execute RISC0 aggregation in mock mode: {:?}", e);
                    RaikoError::Guest(format!("RISC0 aggregation mock execution failed: {e}"))
                })?;
                let claim = session.receipt_claim.clone().ok_or_else(|| {
                    RaikoError::Guest(
                        "RISC0 aggregation mock execution returned no receipt claim".to_string(),
                    )
                })?;
                let receipt = Receipt::try_from(FakeReceipt::new(claim)).map_err(|e| {
                    RaikoError::Guest(format!(
                        "Failed to convert RISC0 aggregation mock receipt: {e}"
                    ))
                })?;

                let journal_bytes = &receipt.journal.bytes;
                let agg_input_hash = if journal_bytes.len() >= 32 {
                    B256::from_slice(&journal_bytes[..32])
                } else {
                    B256::default()
                };
                let extra_data = Self::mock_extra_data(
                    &session,
                    image_id,
                    agg_input_hash,
                    journal_bytes.len(),
                    "mock",
                )?;
                (receipt, extra_data)
            } else {
                let receipt = default_prover()
                    .prove_with_opts(env, &elf, &opts)
                    .map_err(|e| {
                        tracing::error!("Failed to generate RISC0 aggregation proof: {:?}", e);
                        RaikoError::Guest(format!("RISC0 aggregation proof generation failed: {e}"))
                    })?
                    .receipt;
                (receipt, None)
            };

            info!("RISC0 aggregation proof generated successfully");
            if prover_config.mock {
                info!("RISC0 mock mode enabled; aggregation receipt is fake but journal is real");
            }
            Risc0Prover {
                config: prover_config.clone(),
            }
            .verify_receipt(&receipt, image_id)?;
            if prover_config.verify {
                info!("RISC0 aggregation proof verified successfully");
            }

            let journal_bytes = &receipt.journal.bytes;
            let agg_input_hash = if journal_bytes.len() >= 32 {
                B256::from_slice(&journal_bytes[..32])
            } else {
                B256::default()
            };

            let receipt_json = serde_json::to_string(&receipt).unwrap_or_default();

            Ok::<Proof, RaikoError>(
                Risc0Response {
                    proof: alloy_primitives::hex::encode_prefixed(journal_bytes),
                    receipt: receipt_json,
                    image_id: alloy_primitives::hex::encode_prefixed(image_id.as_bytes()),
                    input: agg_input_hash,
                    extra_data,
                }
                .into(),
            )
        })
        .await
        .map_err(|e| RaikoError::Guest(format!("RISC0 aggregation proof task join failed: {e}")))?
    }
}
