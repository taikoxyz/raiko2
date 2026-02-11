use super::manifest::ShastaManifestBuilder;
use crate::{ManifestBuilder, PipelineKey, PipelineSpec, Preflight, ProverBackend, Validation};
use alethia_reth_block::config::TaikoEvmConfig;
use alloy_consensus::transaction::SignerRecoverable;
use raiko2_primitives::{
    ChainSpec, ProofContext, RaikoError, RaikoResult, StatelessInput, SupportedChainSpecs,
};
use raiko2_primitives_shasta::GuestInput;
use raiko2_provider::Provider;
use raiko2_stateless::validate_block;
use serde_json::Value;

/// Shasta hardfork specification.
#[derive(Debug)]
pub struct ShastaSpec<Pr, Bk, Pv> {
    prover: Pr,
    backend: Bk,
    provider: Pv,
    manifest_builder: ShastaManifestBuilder,
    pipeline_key: PipelineKey,
}

impl<Pr, Bk, Pv> ShastaSpec<Pr, Bk, Pv> {
    /// Create a Shasta spec with the default manifest builder.
    pub const fn new(pipeline_key: PipelineKey, prover: Pr, backend: Bk, provider: Pv) -> Self {
        Self {
            prover,
            backend,
            provider,
            manifest_builder: ShastaManifestBuilder::new(),
            pipeline_key,
        }
    }

    /// Create a Shasta spec using the provided manifest builder.
    pub const fn with_manifest_builder(
        manifest_builder: ShastaManifestBuilder,
        pipeline_key: PipelineKey,
        prover: Pr,
        backend: Bk,
        provider: Pv,
    ) -> Self {
        Self {
            prover,
            backend,
            provider,
            manifest_builder,
            pipeline_key,
        }
    }
}

#[async_trait::async_trait]
impl<Pr, Bk, Pv> Preflight for ShastaSpec<Pr, Bk, Pv>
where
    Pr: Send + Sync,
    Bk: ProverBackend,
    Pv: Provider,
{
    type Input = GuestInput;

    async fn preflight<P: Provider>(
        &self,
        ctx: &ProofContext,
        provider: &P,
    ) -> RaikoResult<GuestInput> {
        let (block_numbers, expected_proposal_id) = extract_block_range(ctx)?;
        let blocks = provider.batch_blocks(&block_numbers).await?;
        let witnesses = provider.batch_witnesses(&block_numbers).await?;

        // Collect signers for each block to fetch the touched accounts.
        let mut all_signers = Vec::with_capacity(blocks.len());
        for block in &blocks {
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

        validate_block_range(&witnesses, expected_proposal_id)?;

        Ok(GuestInput {
            taiko: manifest,
            witnesses,
        })
    }
}

fn extract_block_range(ctx: &ProofContext) -> RaikoResult<(Vec<u64>, u64)> {
    if let Some(range) = ctx.config.get("l2_block_range") {
        let start = range.get("start").and_then(Value::as_u64).ok_or_else(|| {
            RaikoError::InvalidRequestConfig("l2_block_range.start missing".into())
        })?;
        let end = range
            .get("end")
            .and_then(Value::as_u64)
            .ok_or_else(|| RaikoError::InvalidRequestConfig("l2_block_range.end missing".into()))?;
        if start > end {
            return Err(RaikoError::InvalidRequestConfig(
                "l2_block_range.start must be <= end".into(),
            ));
        }
        let proposal_id = range
            .get("proposal_id")
            .and_then(Value::as_u64)
            .unwrap_or(ctx.request.proposal_id);
        let blocks: Vec<u64> = (start..=end).collect();
        Ok((blocks, proposal_id))
    } else {
        Ok((vec![ctx.request.proposal_id], ctx.request.proposal_id))
    }
}

fn validate_block_range(
    witnesses: &[StatelessInput],
    expected_proposal_id: u64,
) -> RaikoResult<()> {
    if witnesses.is_empty() {
        return Err(RaikoError::Preflight(
            "GuestInput has no witnesses to validate".to_string(),
        ));
    }
    // Proposal id is stored in extradata bytes[1..7] big-endian in Shasta.
    for (idx, w) in witnesses.iter().enumerate() {
        let extradata = &w.block.header.extra_data;
        if extradata.len() < 7 {
            return Err(RaikoError::Preflight(format!(
                "witness {idx} extradata too short ({})",
                extradata.len()
            )));
        }
        let pid_bytes = &extradata[1..7];
        let pid = u64::from_be_bytes([
            0,
            0,
            0,
            pid_bytes[0],
            pid_bytes[1],
            pid_bytes[2],
            pid_bytes[3],
            pid_bytes[4],
        ]);
        let pid = (pid << 8) | u64::from(pid_bytes[5]);
        if pid != expected_proposal_id {
            return Err(RaikoError::Preflight(format!(
                "witness {idx} proposal_id mismatch: expected {expected_proposal_id}, got {pid}"
            )));
        }
    }

    // Blocks must be contiguous.
    for pair in witnesses.windows(2).enumerate() {
        let (idx, window) = pair;
        let prev = window[0].block.header.number;
        let next = window[1].block.header.number;
        if next != prev + 1 {
            return Err(RaikoError::Preflight(format!(
                "witness blocks not contiguous at index {idx}: {prev} -> {next}"
            )));
        }
    }

    Ok(())
}

impl<Pr, Bk, Pv> Validation for ShastaSpec<Pr, Bk, Pv>
where
    Pr: Send + Sync,
    Bk: ProverBackend,
    Pv: Provider,
{
    type Input = GuestInput;

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
                &stateless_input.witness,
                stateless_input.accounts.clone(),
                &taiko_chain_spec,
                &evm_config,
            )?;
        }

        Ok(())
    }
}

impl<Pr, Bk, Pv> PipelineSpec for ShastaSpec<Pr, Bk, Pv>
where
    Pr: Send + Sync,
    Bk: ProverBackend,
    Pv: Provider,
{
    type GuestInput = GuestInput;
    type Preflight = Self;
    type Validation = Self;
    type ManifestBuilder = ShastaManifestBuilder;
    type Prover = Pr;
    type Backend = Bk;
    type Provider = Pv;

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
        self
    }

    fn manifest_builder(&self) -> &Self::ManifestBuilder {
        &self.manifest_builder
    }
}
