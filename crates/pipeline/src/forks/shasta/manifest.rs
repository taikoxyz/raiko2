use crate::ManifestBuilder;
use alethia_reth_consensus::validation::ANCHOR_V3_V4_GAS_LIMIT;
use raiko2_primitives::{ChainSpec, ProofContext, RaikoError, RaikoResult, SupportedChainSpecs};
use raiko2_protocol::InputDataSource;
use raiko2_protocol::ManifestChainSpec;
use raiko2_protocol_shasta::shasta::{
    Proposed, ShastaEventData, manifest::DerivationSourceManifest,
};
use raiko2_protocol_shasta::{TaikoManifest, TaikoProverData};
use reth_ethereum_primitives::Block;
use serde_json::Value;
use tracing::info;

/// Shasta manifest builder.
#[derive(Debug, Clone, Default)]
pub struct ShastaManifestBuilder;

impl ShastaManifestBuilder {
    /// Create a new Shasta manifest builder.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    fn parse_prover_data(ctx: &ProofContext) -> TaikoProverData {
        let prover_address = ctx
            .request
            .prover
            .as_ref()
            .and_then(|s| s.parse().ok())
            .unwrap_or_default();

        TaikoProverData {
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
            last_anchor_block_number: ctx
                .request
                .shasta
                .map(|shasta| shasta.last_anchor_block_number),
        }
    }

    fn build_chain_spec(ctx: &ProofContext) -> ManifestChainSpec {
        let chain_spec = SupportedChainSpecs::default()
            .get_chain_spec_with_chain_id(ctx.request.l2_chain_id)
            .unwrap_or_else(|| ChainSpec {
                name: "unknown".to_string(),
                chain_id: ctx.request.l2_chain_id,
                is_taiko: true,
                ..Default::default()
            });
        ManifestChainSpec {
            name: chain_spec.name,
            chain_id: chain_spec.chain_id,
            is_taiko: chain_spec.is_taiko,
        }
    }

    fn parse_proposal_event(ctx: &ProofContext) -> RaikoResult<ShastaEventData> {
        if let Some(event) = Self::parse_config(&ctx.config, "shasta_proposal_event")? {
            return Ok(event);
        }
        if let Some(proposed) =
            Self::parse_config::<Proposed>(&ctx.config, "shasta_proposed_event")?
        {
            return ShastaEventData::from_proposal_event(&proposed).map_err(|err| {
                RaikoError::InvalidRequestConfig(format!("invalid shasta_proposed_event: {err}"))
            });
        }
        Ok(ShastaEventData::default())
    }

    fn resolve_manifest_payload(
        manifest_payload: Option<Vec<u8>>,
        manifest_payloads: Option<&Vec<Vec<u8>>>,
    ) -> Option<Vec<u8>> {
        manifest_payload.or_else(|| {
            manifest_payloads.and_then(|payloads| {
                if payloads.is_empty() {
                    None
                } else {
                    let mut concatenated = Vec::new();
                    for payload in payloads {
                        concatenated.extend_from_slice(payload);
                    }
                    Some(concatenated)
                }
            })
        })
    }

    fn derive_data_sources(
        proposal_event: &ShastaEventData,
        payload: &[u8],
        manifest_payloads: Option<&Vec<Vec<u8>>>,
    ) -> Vec<InputDataSource> {
        let sources = &proposal_event.proposal.sources;
        if sources.is_empty() {
            let tx_data_from_blob = if let Some(payloads) = manifest_payloads {
                payloads.clone()
            } else {
                vec![payload.to_vec()]
            };
            return vec![InputDataSource {
                tx_data_from_blob,
                is_forced_inclusion: false,
                ..Default::default()
            }];
        }

        if sources.len() == 1 {
            let tx_data_from_blob = if let Some(payloads) = manifest_payloads {
                payloads.clone()
            } else {
                vec![payload.to_vec()]
            };
            return vec![InputDataSource {
                tx_data_from_blob,
                is_forced_inclusion: sources[0].isForcedInclusion,
                ..Default::default()
            }];
        }

        if let Some(payloads) = manifest_payloads {
            let mut cursor = 0usize;
            let mut data_sources = Vec::with_capacity(sources.len());
            for source in sources {
                let expected = source.blobSlice.blobHashes.len();
                let end = cursor.saturating_add(expected).min(payloads.len());
                let blob_payloads = if cursor < end {
                    payloads[cursor..end].to_vec()
                } else {
                    Vec::new()
                };
                cursor = end;
                data_sources.push(InputDataSource {
                    tx_data_from_blob: blob_payloads,
                    is_forced_inclusion: source.isForcedInclusion,
                    ..Default::default()
                });
            }
            return data_sources;
        }

        let tx_data_from_blob = vec![payload.to_vec()];
        sources
            .iter()
            .enumerate()
            .map(|(index, source)| {
                let blob_payloads = if index == sources.len() - 1 {
                    tx_data_from_blob.clone()
                } else {
                    Vec::new()
                };
                InputDataSource {
                    tx_data_from_blob: blob_payloads,
                    is_forced_inclusion: source.isForcedInclusion,
                    ..Default::default()
                }
            })
            .collect()
    }

    fn parse_config<T>(config: &Value, key: &str) -> RaikoResult<Option<T>>
    where
        T: serde::de::DeserializeOwned,
    {
        let Some(value) = config.get(key) else {
            return Ok(None);
        };
        T::deserialize(value)
            .map(Some)
            .map_err(|e| RaikoError::InvalidRequestConfig(format!("invalid {key}: {e}")))
    }

    fn validate_manifest_against_blocks(
        manifest: &DerivationSourceManifest,
        blocks: &[Block],
    ) -> RaikoResult<()> {
        if manifest.blocks.is_empty() {
            return Err(RaikoError::InvalidRequestConfig(
                "shasta manifest contains no blocks".to_string(),
            ));
        }

        if manifest.blocks.len() != blocks.len() {
            return Err(RaikoError::InvalidRequestConfig(format!(
                "shasta manifest blocks length {} does not match input blocks length {}",
                manifest.blocks.len(),
                blocks.len()
            )));
        }

        for (index, (manifest_block, block)) in manifest.blocks.iter().zip(blocks).enumerate() {
            if manifest_block.timestamp != block.header.timestamp {
                return Err(RaikoError::InvalidRequestConfig(format!(
                    "shasta manifest block {index} timestamp mismatch"
                )));
            }
            if manifest_block.coinbase != block.header.beneficiary {
                return Err(RaikoError::InvalidRequestConfig(format!(
                    "shasta manifest block {index} coinbase mismatch"
                )));
            }

            let expected_gas_limit = if block.header.number == 0 {
                block.header.gas_limit
            } else {
                manifest_block.gas_limit + ANCHOR_V3_V4_GAS_LIMIT
            };
            if expected_gas_limit != block.header.gas_limit {
                return Err(RaikoError::InvalidRequestConfig(format!(
                    "shasta manifest block {index} gas limit mismatch"
                )));
            }
        }

        Ok(())
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
        Self::build_taiko_manifest(ctx, blocks, None)
    }
}

impl ShastaManifestBuilder {
    /// Build a Shasta manifest using an already-resolved proposal event.
    ///
    /// # Errors
    ///
    /// Returns an error if manifest config or payload data is invalid.
    pub fn taiko_manifest_with_event(
        ctx: &ProofContext,
        blocks: &[Block],
        proposal_event: ShastaEventData,
    ) -> RaikoResult<TaikoManifest> {
        Self::build_taiko_manifest(ctx, blocks, Some(proposal_event))
    }

    fn build_taiko_manifest(
        ctx: &ProofContext,
        blocks: &[Block],
        proposal_event_override: Option<ShastaEventData>,
    ) -> RaikoResult<TaikoManifest> {
        info!(
            "Creating Taiko manifest for proposal {} with {} blocks",
            ctx.request.proposal_id,
            blocks.len()
        );

        // TODO: Implement actual L1 proposal fetching using raiko2-protocol.
        let prover_data = Self::parse_prover_data(ctx);
        let chain_spec = Self::build_chain_spec(ctx);

        let l1_header = Self::parse_config(&ctx.config, "l1_header")?
            .unwrap_or_else(alloy_consensus::Header::default);
        let l1_ancestor_headers =
            Self::parse_config::<Vec<alloy_consensus::Header>>(&ctx.config, "l1_ancestor_headers")?
                .unwrap_or_default();
        let proposal_event =
            proposal_event_override.map_or_else(|| Self::parse_proposal_event(ctx), Ok)?;
        let mut data_sources =
            Self::parse_config::<Vec<InputDataSource>>(&ctx.config, "shasta_data_sources")?
                .unwrap_or_default();
        let manifest_offset =
            Self::parse_config::<usize>(&ctx.config, "shasta_manifest_offset")?.unwrap_or(0);
        let manifest_payload =
            Self::parse_config::<Vec<u8>>(&ctx.config, "shasta_manifest_payload")?;
        let manifest_payloads =
            Self::parse_config::<Vec<Vec<u8>>>(&ctx.config, "shasta_manifest_blob_payloads")?;
        let manifest_payload =
            Self::resolve_manifest_payload(manifest_payload, manifest_payloads.as_ref());

        if let Some(payload) = &manifest_payload {
            let manifest = DerivationSourceManifest::decompress_and_decode(
                payload,
                manifest_offset,
            )
            .map_err(|err| {
                RaikoError::InvalidRequestConfig(format!("invalid shasta manifest payload: {err}"))
            })?;
            info!(
                blocks = manifest.blocks.len(),
                "decoded shasta manifest payload"
            );

            let validate_manifest =
                Self::parse_config::<bool>(&ctx.config, "shasta_validate_manifest")?
                    .unwrap_or(true);
            if validate_manifest {
                Self::validate_manifest_against_blocks(&manifest, blocks)?;
            }

            if data_sources.is_empty() {
                data_sources =
                    Self::derive_data_sources(&proposal_event, payload, manifest_payloads.as_ref());
            }
        }

        Ok(TaikoManifest {
            proposal_id: ctx.request.proposal_id,
            l1_header,
            proposal_event,
            chain_spec,
            prover_data,
            data_sources,
            l1_ancestor_headers,
        })
    }
}
