//! RISC0 zkVM Prover for Raiko V2
//!
//! This module provides the RISC0 prover implementation for generating
//! zero-knowledge proofs of Taiko block execution.

mod types;

pub use types::{Risc0Config, Risc0ExecutionMetadata, Risc0Response};

use alloy_primitives::{B256, Bytes};
use raiko2_pipeline::{ProofStage, ProverBackend};
use raiko2_primitives::{AggregationGuestInput, Proof, ProverConfig, RaikoError, RaikoResult};
use raiko2_primitives_shasta::GuestInput;
use risc0_zkvm::{
    Digest, ExecutorEnv, FakeReceipt, ProverOpts, Receipt, VerifierContext, compute_image_id,
    default_executor, default_prover,
};
use tracing::info;

use crate::{
    GuestInputCodec, encode_risc0_proof_payload, parse_shasta_aggregation_input,
    parse_shasta_aggregation_input_hash, parse_shasta_proof_carry_data,
    parse_shasta_proposal_input_hash, validate_shasta_aggregation_lengths, with_shasta_extra_data,
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
        if !config.is_object() {
            *config = serde_json::json!({});
        }
        let Some(config) = config.as_object_mut() else {
            return Err(RaikoError::InvalidRequestConfig(
                "prover config must be a JSON object".to_string(),
            ));
        };
        config.insert(
            "shasta_proof_carry_data".to_string(),
            serde_json::to_value(&input.proof_carry_data).map_err(|e| {
                RaikoError::InvalidRequestConfig(format!(
                    "Failed to serialize proof carry data: {e}"
                ))
            })?,
        );
        Ok(())
    }

    async fn prove_encoded(
        &self,
        input: Bytes,
        config: &ProverConfig,
        backend: &B,
    ) -> RaikoResult<Proof> {
        info!("Starting RISC0 proposal proof generation...");

        let elf = backend.elf(ProofStage::Proposal)?.to_vec();
        let prover_config = self.config.clone();
        let proof_carry_data = parse_shasta_proof_carry_data(config)?;
        let opts = self.prover_opts();

        tokio::task::spawn_blocking(move || {
            let env = ExecutorEnv::builder()
                .write_frame(input.as_ref())
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
                let input_hash = parse_shasta_proposal_input_hash(journal_bytes)?;
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
            let input_hash = parse_shasta_proposal_input_hash(journal_bytes)?;

            info!(
                "Generated proposal receipt journal: {:?}",
                alloy_primitives::hex::encode_prefixed(journal_bytes.clone())
            );

            let receipt_json = serde_json::to_string(&receipt).unwrap_or_default();

            Ok::<Proof, RaikoError>(
                Risc0Response {
                    proof: encode_risc0_proof_payload(&receipt),
                    receipt: receipt_json,
                    image_id: alloy_primitives::hex::encode_prefixed(image_id.as_bytes()),
                    input: input_hash,
                    extra_data: with_shasta_extra_data(&proof_carry_data, "risc0", extra_data)?,
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
                let agg_input_hash = parse_shasta_aggregation_input_hash(journal_bytes);
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
            let agg_input_hash = parse_shasta_aggregation_input_hash(journal_bytes);

            let receipt_json = serde_json::to_string(&receipt).unwrap_or_default();

            Ok::<Proof, RaikoError>(
                Risc0Response {
                    proof: encode_risc0_proof_payload(&receipt),
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

#[cfg(test)]
mod tests {
    use super::{Risc0Config, Risc0Prover};
    use crate::Prover;
    use alloy_consensus::{SignableTransaction, TxEip1559};
    use alloy_primitives::{Address, B256, Signature, TxKind, U256};
    use alloy_sol_types::{SolCall, sol};
    use raiko2_pipeline::forks::shasta::RISC0_SHASTA_BACKEND;
    use raiko2_primitives::{ProofType, ProverConfig, SupportedChainSpecs};
    use raiko2_primitives_shasta::{GuestInput, build_proof_carry_data};
    use raiko2_protocol_shasta::TaikoManifest;
    sol! {
        #[derive(Debug)]
        struct AnchorV4Checkpoint {
            uint48 blockNumber;
            bytes32 blockHash;
            bytes32 stateRoot;
        }

        function anchorV4(AnchorV4Checkpoint _checkpoint) external;
    }

    fn anchor_tx(checkpoint: &AnchorV4Checkpoint) -> reth_ethereum_primitives::TransactionSigned {
        TxEip1559 {
            chain_id: 167_000,
            nonce: 0,
            gas_limit: 1_000_000,
            max_fee_per_gas: 1,
            max_priority_fee_per_gas: 0,
            to: TxKind::Call(Address::ZERO),
            value: U256::ZERO,
            access_list: Default::default(),
            input: anchorV4Call {
                _checkpoint: checkpoint.clone(),
            }
            .abi_encode()
            .into(),
        }
        .into_signed(Signature::test_signature())
        .into()
    }

    fn fixture_guest_input() -> GuestInput {
        let chain_spec = SupportedChainSpecs::default()
            .get_chain_spec_with_chain_id(167_000)
            .expect("supported taiko mainnet chain spec");
        let mut witness = raiko2_primitives::StatelessInput {
            chain_spec,
            ..Default::default()
        };
        witness.block.header.number = 1;
        witness.block.header.timestamp = u64::MAX / 2;
        witness.block.header.parent_hash = B256::from([9u8; 32]);
        witness.block.header.state_root = B256::from([1u8; 32]);

        let mut l1_header = alloy_consensus::Header::default();
        l1_header.number = 7;
        l1_header.parent_hash = B256::from([0xAA; 32]);
        l1_header.state_root = B256::from([0x66; 32]);
        let checkpoint = AnchorV4Checkpoint {
            blockNumber: l1_header.number.try_into().expect("fits in uint48"),
            blockHash: l1_header.hash_slow(),
            stateRoot: l1_header.state_root,
        };
        witness.block.body.transactions.push(anchor_tx(&checkpoint));

        let mut input = GuestInput {
            witnesses: vec![witness],
            taiko: TaikoManifest {
                proposal_id: 42,
                ..Default::default()
            },
            ..Default::default()
        };
        input.taiko.chain_spec.name = "taiko_mainnet".to_string();
        input.taiko.chain_spec.chain_id = 167_000;
        input.taiko.chain_spec.is_taiko = true;
        input.taiko.l1_header = l1_header.clone();
        input.taiko.l1_ancestor_headers = vec![l1_header.clone()];
        input.taiko.prover_data.actual_prover = Address::from([0x22; 20]);
        input.taiko.proposal_event.proposal.id =
            input.taiko.proposal_id.try_into().expect("fits in uint48");
        input.taiko.proposal_event.proposal.proposer = Address::from([0x33; 20]);
        input.taiko.proposal_event.proposal.timestamp =
            123u64.try_into().expect("timestamp fits in uint48");
        input.taiko.proposal_event.proposal.parentProposalHash = B256::from([0x44; 32]);
        input.taiko.proposal_event.proposal.originBlockNumber =
            l1_header.number.try_into().expect("fits in uint48");
        input.taiko.proposal_event.proposal.originBlockHash = l1_header.hash_slow();
        input.proof_carry_data =
            build_proof_carry_data(&input, ProofType::Risc0).expect("build carry data");
        input
    }

    #[tokio::test]
    async fn risc0_mock_proposal_surfaces_guest_validation_errors_after_framed_input() {
        let prover = Risc0Prover::new(Risc0Config {
            bonsai: false,
            snark: false,
            mock: true,
            profile: false,
            execution_po2: 20,
            verify: true,
        });
        let guest_input = fixture_guest_input();

        let err = prover
            .prove(guest_input, &ProverConfig::default(), &RISC0_SHASTA_BACKEND)
            .await
            .expect_err("fixture input should reach guest validation and fail there");

        let message = err.to_string();
        assert!(message.contains("RISC0 proposal mock execution failed"));
        assert!(message.contains("stateless block validation failed at index 0"));
        assert!(message.contains("missing required ancestor headers"));
    }
}
