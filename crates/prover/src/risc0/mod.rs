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
    default_executor, get_prover_server,
};
use tracing::info;

use crate::{
    GuestInputCodec, encode_risc0_aggregation_proof_payload, encode_risc0_proposal_proof_payload,
    ensure_proposal_input_matches_carry, parse_proposal_aggregation_input_hash,
    parse_proposal_input_hash, risc0_aggregation::build_risc0_aggregation_input_from_proofs,
    with_proposal_extra_data,
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

    fn build_framed_env(input: &[u8], execution_po2: u32) -> RaikoResult<ExecutorEnv<'_>> {
        let mut env_builder = ExecutorEnv::builder();
        env_builder
            .write_frame(input)
            .segment_limit_po2(execution_po2);
        env_builder
            .build()
            .map_err(|e| RaikoError::Guest(format!("Failed to build env: {e}")))
    }

    fn compute_image_id(elf: &[u8]) -> RaikoResult<Digest> {
        compute_image_id(elf)
            .map_err(|e| RaikoError::Guest(format!("Failed to compute image id: {e}")))
    }

    fn image_id_hex(image_id: Digest) -> String {
        alloy_primitives::hex::encode_prefixed(image_id.as_bytes())
    }

    fn execute_real_proof(
        env: ExecutorEnv<'_>,
        elf: &[u8],
        opts: &ProverOpts,
        stage: &str,
    ) -> RaikoResult<Receipt> {
        let ctx = VerifierContext::default().with_dev_mode(opts.dev_mode());
        get_prover_server(opts)
            .map_err(|e| {
                RaikoError::Guest(format!(
                    "Failed to initialize local RISC0 prover server: {e}"
                ))
            })?
            .prove_with_ctx(env, &ctx, elf)
            .map(|info| info.receipt)
            .map_err(|e| {
                tracing::error!("Failed to generate RISC0 {} proof: {:?}", stage, e);
                RaikoError::Guest(format!("RISC0 {stage} proof generation failed: {e}"))
            })
    }

    fn execute_mock_proof<F>(
        env: ExecutorEnv<'_>,
        elf: &[u8],
        image_id: Digest,
        stage: &str,
        input_hash: F,
    ) -> RaikoResult<(Receipt, Option<serde_json::Value>)>
    where
        F: FnOnce(&[u8]) -> RaikoResult<B256>,
    {
        let session = default_executor().execute(env, elf).map_err(|e| {
            tracing::error!("Failed to execute RISC0 {} in mock mode: {:?}", stage, e);
            RaikoError::Guest(format!("RISC0 {stage} mock execution failed: {e}"))
        })?;
        let claim = session.receipt_claim.clone().ok_or_else(|| {
            RaikoError::Guest(format!(
                "RISC0 {stage} mock execution returned no receipt claim"
            ))
        })?;
        let receipt = Receipt::try_from(FakeReceipt::new(claim)).map_err(|e| {
            RaikoError::Guest(format!("Failed to convert RISC0 {stage} mock receipt: {e}"))
        })?;

        let journal_bytes = &receipt.journal.bytes;
        let extra_data = Self::mock_extra_data(
            &session,
            image_id,
            input_hash(journal_bytes)?,
            journal_bytes.len(),
            "mock",
        )?;
        Ok((receipt, extra_data))
    }

    fn execute_proof<F>(
        &self,
        env: ExecutorEnv<'_>,
        elf: &[u8],
        opts: &ProverOpts,
        image_id: Digest,
        stage: &str,
        input_hash: F,
    ) -> RaikoResult<(Receipt, Option<serde_json::Value>)>
    where
        F: FnOnce(&[u8]) -> RaikoResult<B256>,
    {
        if self.config.mock {
            Self::execute_mock_proof(env, elf, image_id, stage, input_hash)
        } else {
            Self::execute_real_proof(env, elf, opts, stage).map(|receipt| (receipt, None))
        }
    }

    fn finalize_stage(&self, stage: &str, receipt: &Receipt, image_id: Digest) -> RaikoResult<()> {
        info!("RISC0 {} proof generated successfully", stage);
        if self.config.mock {
            info!(
                "RISC0 mock mode enabled; {} receipt is fake but journal is real",
                stage
            );
        }
        self.verify_receipt(receipt, image_id)?;
        if self.config.verify {
            info!("RISC0 {} proof verified successfully", stage);
        }
        Ok(())
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

    async fn prove_encoded(
        &self,
        input: Bytes,
        _config: &ProverConfig,
        backend: &B,
    ) -> RaikoResult<Proof> {
        info!("Starting RISC0 proposal proof generation...");

        let elf = backend.elf(ProofStage::Proposal)?.to_vec();
        let prover_config = self.config.clone();
        let guest_input: GuestInput = bincode::deserialize(input.as_ref())
            .map_err(|e| RaikoError::Guest(format!("Failed to deserialize input: {e}")))?;
        let proof_carry_data = guest_input.proof_carry_data;
        let opts = self.prover_opts();

        tokio::task::spawn_blocking(move || {
            let prover = Risc0Prover::new(prover_config);
            let env = Self::build_framed_env(input.as_ref(), prover.config.execution_po2)?;
            let image_id = Self::compute_image_id(&elf)?;
            let (receipt, extra_data) = prover.execute_proof(
                env,
                &elf,
                &opts,
                image_id,
                "proposal",
                parse_proposal_input_hash,
            )?;
            prover.finalize_stage("proposal", &receipt, image_id)?;

            let journal_bytes = &receipt.journal.bytes;
            let input_hash = parse_proposal_input_hash(journal_bytes)?;
            ensure_proposal_input_matches_carry(input_hash, &proof_carry_data, "risc0")?;

            info!(
                "Generated proposal receipt journal: {:?}",
                alloy_primitives::hex::encode_prefixed(journal_bytes)
            );

            let receipt_json = serde_json::to_string(&receipt).unwrap_or_default();

            Ok::<Proof, RaikoError>(
                Risc0Response {
                    proof: encode_risc0_proposal_proof_payload(
                        &receipt,
                        B256::from_slice(image_id.as_bytes()),
                    ),
                    receipt: receipt_json,
                    image_id: Self::image_id_hex(image_id),
                    input: input_hash,
                    extra_data: with_proposal_extra_data(&proof_carry_data, "risc0", extra_data)?,
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
        _config: &ProverConfig,
        backend: &B,
    ) -> RaikoResult<Proof> {
        info!(
            "Starting RISC0 aggregation proof generation with {} proofs...",
            input.proofs.len()
        );

        let proposal_elf = backend.elf(ProofStage::Proposal)?.to_vec();
        let elf = backend.elf(ProofStage::Aggregation)?.to_vec();
        let prover_config = self.config.clone();
        let opts = self.prover_opts();

        tokio::task::spawn_blocking(move || {
            let prover = Risc0Prover::new(prover_config);
            let proposal_image_id = Self::compute_image_id(&proposal_elf)?;
            let block_image_id = B256::from_slice(proposal_image_id.as_bytes());
            let aggregation_input =
                build_risc0_aggregation_input_from_proofs(input.proofs, proposal_image_id)?;
            let env = Self::build_framed_env(&aggregation_input, prover.config.execution_po2)?;
            let image_id = Self::compute_image_id(&elf)?;
            let (receipt, extra_data) = prover.execute_proof(
                env,
                &elf,
                &opts,
                image_id,
                "aggregation",
                parse_proposal_aggregation_input_hash,
            )?;
            prover.finalize_stage("aggregation", &receipt, image_id)?;

            let journal_bytes = &receipt.journal.bytes;
            let agg_input_hash = parse_proposal_aggregation_input_hash(journal_bytes)?;

            let receipt_json = serde_json::to_string(&receipt).unwrap_or_default();

            Ok::<Proof, RaikoError>(
                Risc0Response {
                    proof: encode_risc0_aggregation_proof_payload(
                        &receipt,
                        block_image_id,
                        B256::from_slice(image_id.as_bytes()),
                    ),
                    receipt: receipt_json,
                    image_id: Self::image_id_hex(image_id),
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
    use alloy::eips::eip2930::AccessList;
    use alloy_consensus::{SignableTransaction, TxEip1559};
    use alloy_primitives::{Address, B256, Signature, TxKind, U256};
    use alloy_sol_types::{SolCall, sol};
    use raiko2_pipeline::proposal::load_risc0_proposal_backend;
    use raiko2_primitives::{ProofType, ProverConfig, SupportedChainSpecs, WitnessHeader};
    use raiko2_primitives_shasta::{GuestInput, build_proof_carry_data_from_witness_spec};
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
            access_list: AccessList::default(),
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
        let parent_header = alloy_consensus::Header {
            number: 0,
            timestamp: 1,
            parent_hash: B256::from([8u8; 32]),
            state_root: B256::from([0x11; 32]),
            ..Default::default()
        };

        let mut witness = raiko2_primitives::StatelessInput {
            chain_spec,
            ..Default::default()
        };
        witness.block.header.number = 1;
        witness.block.header.timestamp = u64::MAX / 2;
        witness.block.header.parent_hash = parent_header.hash_slow();
        witness.block.header.state_root = B256::from([1u8; 32]);
        witness.witness.headers = vec![WitnessHeader::from_header(parent_header)];

        let l1_header = alloy_consensus::Header {
            number: 7,
            parent_hash: B256::from([0xAA; 32]),
            state_root: B256::from([0x66; 32]),
            ..Default::default()
        };
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
        input.proof_carry_data = build_proof_carry_data_from_witness_spec(&input, ProofType::Risc0)
            .expect("build carry data");
        input
    }

    #[tokio::test]
    async fn risc0_mock_proposal_surfaces_guest_validation_errors_after_framed_input() {
        let prover = Risc0Prover::new(Risc0Config {
            snark: false,
            mock: true,
            profile: false,
            execution_po2: 20,
            verify: true,
        });
        let guest_input = fixture_guest_input();
        let backend = load_risc0_proposal_backend().expect("load RISC0 Shasta guest ELFs");

        let err = prover
            .prove(guest_input, &ProverConfig::default(), &backend)
            .await
            .expect_err("fixture input should reach guest validation and fail there");

        let message = err.to_string();
        assert!(
            message.contains("RISC0 proposal mock execution failed"),
            "{message}"
        );
        assert!(
            message.contains("stateless block validation failed at index 0")
                || message.contains("missing expected Shasta block at index 0")
                || message.contains("MPT: Unresolved node access")
                || message.contains("Unresolved node access"),
            "{message}"
        );
    }
}
