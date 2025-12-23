use crate::{
    ManifestBuilder, PipelineSpec, Preflight, Risc0ShastaBackend, Sp1ShastaBackend, Validation,
};
use alethia_reth_block_lite::config::TaikoEvmConfig;
use alloy_consensus::transaction::SignerRecoverable;
use raiko2_guests::{risc0, sp1};
use raiko2_primitives::{
    ChainSpec, GuestInput, ProofContext, RaikoError, RaikoResult, StatelessInput,
    SupportedChainSpecs, TaikoManifest, TaikoProverData,
};
use raiko2_protocol::ShastaEventData;
use raiko2_provider::Provider;
use raiko2_stateless::validate_block;
use reth_ethereum_primitives::Block;
use tracing::info;

/// Shasta manifest builder.
#[derive(Debug, Clone, Default)]
pub struct ShastaManifestBuilder;

impl ShastaManifestBuilder {
    /// Create a new Shasta manifest builder.
    pub const fn new() -> Self {
        Self
    }
}

/// Shasta hardfork specification.
#[derive(Debug, Clone, Default)]
pub struct ShastaSpec {
    manifest_builder: ShastaManifestBuilder,
}

impl ShastaSpec {
    /// Create a Shasta spec using the provided manifest builder.
    pub const fn new(manifest_builder: ShastaManifestBuilder) -> Self {
        Self { manifest_builder }
    }
}

/// RISC0 backend preloaded with Shasta ELFs.
pub const RISC0_SHASTA_BACKEND: Risc0ShastaBackend =
    Risc0ShastaBackend::new(risc0::shasta::PROPOSAL_ELF, risc0::shasta::AGGREGATION_ELF);

/// SP1 backend preloaded with Shasta ELFs.
pub const SP1_SHASTA_BACKEND: Sp1ShastaBackend =
    Sp1ShastaBackend::new(sp1::shasta::PROPOSAL_ELF, sp1::shasta::AGGREGATION_ELF);

#[async_trait::async_trait]
impl ManifestBuilder for ShastaManifestBuilder {
    async fn taiko_manifest(
        &self,
        ctx: &ProofContext,
        blocks: &[Block],
    ) -> RaikoResult<TaikoManifest> {
        info!(
            "Creating Taiko manifest for proposal {} with {} blocks",
            ctx.request.proposal_id,
            blocks.len()
        );

        // TODO: Implement actual L1 proposal fetching using raiko2-protocol.
        let prover_address = ctx
            .request
            .prover
            .as_ref()
            .and_then(|s| s.parse().ok())
            .unwrap_or_default();

        let prover_data = TaikoProverData {
            actual_prover: prover_address,
            designated_prover: None,
            graffiti: ctx
                .request
                .graffiti
                .as_ref()
                .and_then(|s| s.parse().ok())
                .unwrap_or_default(),
            parent_transition_hash: None,
            checkpoint: None,
            last_anchor_block_number: None,
        };

        Ok(TaikoManifest {
            proposal_id: ctx.request.proposal_id,
            l1_header: alloy_consensus::Header::default(),
            proposal_event: ShastaEventData::default(),
            chain_spec: Default::default(),
            prover_data,
            data_sources: Vec::new(),
        })
    }
}

#[async_trait::async_trait]
impl Preflight for ShastaSpec {
    async fn preflight<P: Provider>(
        &self,
        ctx: &ProofContext,
        provider: &P,
    ) -> RaikoResult<GuestInput> {
        let block_numbers = vec![ctx.request.proposal_id];
        let blocks = provider.batch_blocks(&block_numbers).await?;
        let witnesses = provider.batch_witnesses(&block_numbers).await?;

        // Collect signers for each block to fetch the touched accounts.
        let mut all_signers = Vec::with_capacity(blocks.len());
        for block in blocks.iter() {
            let mut signers = Vec::new();
            for tx in block.body.transactions() {
                if let Ok(signer) = tx.recover_signer() {
                    signers.push(signer);
                }
            }
            all_signers.push(signers);
        }

        let accounts = provider
            .batch_accounts(&block_numbers, &all_signers)
            .await?;

        if blocks.len() != witnesses.len() || blocks.len() != accounts.len() {
            return Err(RaikoError::InvalidRequestConfig(
                "Provider returned mismatched input lengths".to_string(),
            ));
        }

        let chain_spec = SupportedChainSpecs::default()
            .get_chain_spec_with_chain_id(ctx.request.l2_chain_id)
            .unwrap_or_else(|| ChainSpec {
                name: "unknown".to_string(),
                chain_id: ctx.request.l2_chain_id,
                ..Default::default()
            });

        let manifest = self.manifest_builder.taiko_manifest(ctx, &blocks).await?;

        let witnesses = blocks
            .into_iter()
            .zip(witnesses)
            .zip(accounts)
            .map(|((block, witness), accounts)| StatelessInput {
                block,
                chain_spec: chain_spec.clone(),
                witness,
                accounts,
            })
            .collect::<Vec<_>>();

        Ok(GuestInput {
            taiko: manifest,
            witnesses,
        })
    }
}

impl Validation for ShastaSpec {
    fn validate(&self, _ctx: &ProofContext, input: &GuestInput) -> RaikoResult<()> {
        let Some(first_input) = input.witnesses.first() else {
            return Err(RaikoError::Preflight(
                "GuestInput has no witnesses to validate".to_string(),
            ));
        };

        let chain_spec = &first_input.chain_spec;
        let taiko_chain_spec = chain_spec
            .to_taiko_chain_spec()
            .map_err(|e| RaikoError::Preflight(e.to_string()))?;
        let evm_config = TaikoEvmConfig::new(taiko_chain_spec.clone());

        for (index, stateless_input) in input.witnesses.iter().enumerate() {
            if stateless_input.chain_spec.chain_id != chain_spec.chain_id {
                return Err(RaikoError::Preflight(format!(
                    "witness {index} chain_id mismatch: expected {}, got {}",
                    chain_spec.chain_id, stateless_input.chain_spec.chain_id
                )));
            }

            if stateless_input.chain_spec.is_taiko != chain_spec.is_taiko {
                return Err(RaikoError::Preflight(format!(
                    "witness {index} is_taiko mismatch: expected {}, got {}",
                    chain_spec.is_taiko, stateless_input.chain_spec.is_taiko
                )));
            }

            validate_block(
                stateless_input.block.clone(),
                stateless_input.witness.clone(),
                stateless_input.accounts.clone(),
                taiko_chain_spec.clone(),
                evm_config.clone(),
            )?;
        }

        Ok(())
    }
}

impl PipelineSpec for ShastaSpec {
    type Preflight = Self;
    type Validation = Self;
    type ManifestBuilder = ShastaManifestBuilder;

    fn preflight(&self) -> &Self::Preflight {
        self
    }

    fn validation(&self) -> &Self::Validation {
        self
    }

    fn manifest_builder(&self) -> &Self::ManifestBuilder {
        &self.manifest_builder
    }
}

// ELF selection is handled by the backend instance.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ProofStage, ProverBackend};

    #[test]
    fn shasta_backends_return_expected_elves() {
        let risc0_proposal = RISC0_SHASTA_BACKEND
            .elf(ProofStage::Proposal)
            .expect("risc0 proposal elf");
        let risc0_agg = RISC0_SHASTA_BACKEND
            .elf(ProofStage::Aggregation)
            .expect("risc0 aggregation elf");
        assert_eq!(risc0_proposal, risc0::shasta::PROPOSAL_ELF);
        assert_eq!(risc0_agg, risc0::shasta::AGGREGATION_ELF);

        let sp1_proposal = SP1_SHASTA_BACKEND
            .elf(ProofStage::Proposal)
            .expect("sp1 proposal elf");
        let sp1_agg = SP1_SHASTA_BACKEND
            .elf(ProofStage::Aggregation)
            .expect("sp1 aggregation elf");
        assert_eq!(sp1_proposal, sp1::shasta::PROPOSAL_ELF);
        assert_eq!(sp1_agg, sp1::shasta::AGGREGATION_ELF);
    }
}
