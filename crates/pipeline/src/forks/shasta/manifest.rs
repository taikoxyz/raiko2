use crate::ManifestBuilder;
use raiko2_primitives::{ProofContext, RaikoResult};
use raiko2_protocol_shasta::shasta::ShastaEventData;
use raiko2_protocol_shasta::{TaikoManifest, TaikoProverData};
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

#[async_trait::async_trait]
impl ManifestBuilder for ShastaManifestBuilder {
    type Manifest = TaikoManifest;

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
