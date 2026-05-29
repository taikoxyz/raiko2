//! Shared Shasta derivation helpers aligned with taiko-mono-rs.

use super::{
    BlobCoder, DerivationSource,
    constants::{
        BLOCK_GAS_LIMIT_MAX_CHANGE, GAS_LIMIT_DENOMINATOR, MAX_BLOCK_GAS_LIMIT,
        MIN_BLOCK_GAS_LIMIT, derivation_source_max_blocks_for_chain_timestamp,
        max_anchor_offset_for_chain, timestamp_max_offset_for_chain,
    },
    manifest::DerivationSourceManifest,
};
use alethia_reth_consensus::validation::ANCHOR_V3_V4_GAS_LIMIT;
use alloy_eips::eip4844::{BYTES_PER_BLOB, Blob};
use alloy_primitives::Address;
use raiko2_protocol::InputDataSource;
use thiserror::Error;

const MAX_MANIFEST_OFFSET: usize = BYTES_PER_BLOB - 64;

/// Metadata shared across all derivation sources in a proposal.
#[derive(Debug, Clone, Copy)]
pub struct ProposalMetadata {
    /// Timestamp of the L1 block that emitted the proposal event.
    pub proposal_timestamp: u64,
    /// L1 block number used as the proposal origin.
    pub origin_block_number: u64,
    /// Proposer address that becomes the inherited coinbase.
    pub proposer: Address,
    /// L2 chain ID used for chain-aware protocol bounds.
    pub chain_id: u64,
}

/// Parent block data required to validate and inherit source metadata.
#[derive(Debug, Clone, Copy)]
pub struct ParentBlockContext {
    /// Timestamp of the parent L2 block.
    pub timestamp: u64,
    /// Gas limit of the parent L2 block, including the reserved anchor gas.
    pub gas_limit: u64,
    /// Number of the parent L2 block.
    pub block_number: u64,
    /// Anchor block number advertised by the parent L2 block.
    pub anchor_block_number: u64,
}

/// Input data required to validate metadata for a single derivation source.
#[derive(Debug, Clone, Copy)]
pub struct ValidationContext {
    /// Timestamp of the parent L2 block.
    pub parent_timestamp: u64,
    /// Gas limit of the parent L2 block (includes the anchor transaction gas when non-genesis).
    pub parent_gas_limit: u64,
    /// Number of the parent L2 block.
    pub parent_block_number: u64,
    /// Anchor block number used by the parent L2 block.
    pub parent_anchor_block_number: u64,
    /// Timestamp provided by the L1 proposal event.
    pub proposal_timestamp: u64,
    /// L1 block number in which the proposal was accepted.
    pub origin_block_number: u64,
    /// Indicates whether the proposal is a forced inclusion.
    pub is_forced_inclusion: bool,
    /// Activation timestamp of the Shasta fork.
    pub fork_timestamp: u64,
    /// L2 chain ID used for chain-aware validation bounds.
    pub chain_id: u64,
}

/// Parameters required to populate inherited metadata for forced/default manifests.
#[derive(Debug, Clone, Copy)]
pub struct InheritedMetadataInput {
    /// Timestamp of the parent L2 block.
    pub parent_timestamp: u64,
    /// Timestamp provided by the L1 proposal event.
    pub proposal_timestamp: u64,
    /// Activation timestamp of the Shasta fork.
    pub fork_timestamp: u64,
    /// Proposer address used as inherited coinbase.
    pub proposer: Address,
    /// Anchor block number used as inherited anchor metadata.
    pub anchor_block_number: u64,
    /// Number of the parent L2 block.
    pub parent_block_number: u64,
    /// Gas limit of the parent L2 block, including anchor gas.
    pub parent_gas_limit: u64,
    /// L2 chain ID used for chain-aware inherited timestamp bounds.
    pub chain_id: u64,
}

/// Errors that can occur during manifest validation.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ValidationError {
    /// Manifest contained no blocks.
    #[error("derivation source manifest contains no blocks")]
    EmptyManifest,
    /// Manifest failed validation and should be defaulted.
    #[error("derivation source manifest failed validation and should be defaulted")]
    DefaultManifest,
}

/// Errors that can occur while preparing a source manifest from host-provided data.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SourceDerivationError {
    /// The host omitted raw blob bytes for a blob-backed source.
    #[error("blob-backed derivation source is missing blob data")]
    MissingBlobData,
    /// A raw blob payload had an unexpected size.
    #[error("blob {index} has invalid length {actual}; expected {expected}")]
    InvalidBlobLength {
        /// Position of the invalid blob in the source.
        index: usize,
        /// Actual payload length.
        actual: usize,
        /// Required blob length.
        expected: usize,
    },
    /// The blob payloads could not be decoded using the Shasta blob codec.
    #[error("blob-backed derivation source has invalid blob encoding")]
    InvalidBlobEncoding,
}

/// Validate a derivation source manifest according to the Shasta metadata rules.
///
/// # Errors
///
/// Returns [`ValidationError::EmptyManifest`] when the manifest has no blocks and
/// [`ValidationError::DefaultManifest`] when the manifest violates Shasta metadata rules and must
/// be replaced with the default/inherited payload.
pub fn validate_source_manifest(
    manifest: &DerivationSourceManifest,
    ctx: &ValidationContext,
) -> Result<(), ValidationError> {
    if block_count(manifest) == 0 {
        return Err(ValidationError::EmptyManifest);
    }

    if !validate_timestamps(
        manifest,
        ctx.parent_timestamp,
        ctx.proposal_timestamp,
        ctx.fork_timestamp,
        ctx.chain_id,
    ) || !validate_anchor_numbers(
        manifest,
        ctx.origin_block_number,
        ctx.parent_anchor_block_number,
        ctx.is_forced_inclusion,
        ctx.chain_id,
    ) || !validate_gas_limit(manifest, ctx.parent_block_number, ctx.parent_gas_limit)
    {
        return Err(ValidationError::DefaultManifest);
    }

    Ok(())
}

/// Return true when the manifest represents the protocol-defined default payload.
#[must_use]
pub fn manifest_is_default(manifest: &DerivationSourceManifest) -> bool {
    if manifest.blocks.len() != 1 {
        return false;
    }

    let block = &manifest.blocks[0];
    block.timestamp == 0
        && block.coinbase == Address::ZERO
        && block.anchor_block_number == 0
        && block.gas_limit == 0
        && block.transactions.is_empty()
}

/// Populate each block with inherited metadata for forced/default manifests.
pub fn apply_inherited_metadata(
    manifest: &mut DerivationSourceManifest,
    input: InheritedMetadataInput,
) {
    let mut parent_ts = input.parent_timestamp;
    let parent_gas_limit =
        effective_parent_gas_limit(input.parent_block_number, input.parent_gas_limit);

    for block in &mut manifest.blocks {
        let lower_bound = compute_timestamp_lower_bound(
            parent_ts,
            input.proposal_timestamp,
            input.fork_timestamp,
            input.chain_id,
        );
        block.timestamp = lower_bound;
        block.coinbase = input.proposer;
        block.anchor_block_number = input.anchor_block_number;
        block.gas_limit = parent_gas_limit;
        parent_ts = lower_bound;
    }
}

/// Decode and sanitize a source manifest using the same default-manifest rules as taiko-mono-rs.
///
/// # Errors
///
/// Returns [`SourceDerivationError`] when a blob-backed source is missing raw blob bytes or when
/// the provided blob bytes cannot be decoded with the shared Shasta blob codec.
pub fn prepare_source_manifest(
    source: &DerivationSource,
    data_source: Option<&InputDataSource>,
    parent: ParentBlockContext,
    meta: ProposalMetadata,
    fork_timestamp: u64,
) -> Result<DerivationSourceManifest, SourceDerivationError> {
    let max_blocks =
        derivation_source_max_blocks_for_chain_timestamp(meta.chain_id, meta.proposal_timestamp);
    prepare_source_manifest_with_max_blocks(
        source,
        data_source,
        parent,
        meta,
        fork_timestamp,
        max_blocks,
    )
}

/// Decode a source manifest for tx-list replay without applying inherited metadata validation.
///
/// This is used by preflight to recover the proposal transaction list that was submitted for a
/// block. Witness replay needs that raw list, while full protocol validation still happens through
/// [`prepare_source_manifest`].
///
/// # Errors
///
/// Returns [`SourceDerivationError`] when a blob-backed source is missing raw blob bytes or when
/// the provided blob bytes cannot be decoded with the shared Shasta blob codec.
pub fn decode_source_manifest_for_tx_list(
    source: &DerivationSource,
    data_source: Option<&InputDataSource>,
    max_blocks: usize,
) -> Result<DerivationSourceManifest, SourceDerivationError> {
    let manifest = if source.blobSlice.blobHashes.is_empty() {
        decode_inline_manifest(
            data_source,
            source.blobSlice.offset.to::<usize>(),
            max_blocks,
        )
    } else if !is_source_offset_valid(source) {
        DerivationSourceManifest::default()
    } else {
        decode_blob_backed_manifest(
            data_source.ok_or(SourceDerivationError::MissingBlobData)?,
            source.blobSlice.offset.to::<usize>(),
            max_blocks,
        )?
    };

    Ok(manifest)
}

/// Decode and sanitize a source manifest using a caller-selected per-source block limit.
///
/// # Errors
///
/// Returns [`SourceDerivationError`] when a blob-backed source is missing raw blob bytes or when
/// the provided blob bytes cannot be decoded with the shared Shasta blob codec.
pub fn prepare_source_manifest_with_max_blocks(
    source: &DerivationSource,
    data_source: Option<&InputDataSource>,
    parent: ParentBlockContext,
    meta: ProposalMetadata,
    fork_timestamp: u64,
    max_blocks: usize,
) -> Result<DerivationSourceManifest, SourceDerivationError> {
    let mut manifest = if source.blobSlice.blobHashes.is_empty() {
        decode_inline_manifest(
            data_source,
            source.blobSlice.offset.to::<usize>(),
            max_blocks,
        )
    } else if !is_source_offset_valid(source) {
        DerivationSourceManifest::default()
    } else {
        decode_blob_backed_manifest(
            data_source.ok_or(SourceDerivationError::MissingBlobData)?,
            source.blobSlice.offset.to::<usize>(),
            max_blocks,
        )?
    };

    if source.isForcedInclusion && manifest.blocks.len() != 1 {
        manifest = DerivationSourceManifest::default();
    }

    if source.isForcedInclusion || manifest_is_default(&manifest) {
        apply_inherited_metadata(
            &mut manifest,
            InheritedMetadataInput {
                parent_timestamp: parent.timestamp,
                proposal_timestamp: meta.proposal_timestamp,
                fork_timestamp,
                proposer: meta.proposer,
                anchor_block_number: parent.anchor_block_number,
                parent_block_number: parent.block_number,
                parent_gas_limit: parent.gas_limit,
                chain_id: meta.chain_id,
            },
        );
    }

    let validation_ctx = ValidationContext {
        parent_timestamp: parent.timestamp,
        parent_gas_limit: parent.gas_limit,
        parent_block_number: parent.block_number,
        parent_anchor_block_number: parent.anchor_block_number,
        proposal_timestamp: meta.proposal_timestamp,
        origin_block_number: meta.origin_block_number,
        is_forced_inclusion: source.isForcedInclusion,
        fork_timestamp,
        chain_id: meta.chain_id,
    };

    match validate_source_manifest(&manifest, &validation_ctx) {
        Ok(()) => Ok(manifest),
        Err(ValidationError::EmptyManifest | ValidationError::DefaultManifest) => {
            let mut default_manifest = DerivationSourceManifest::default();
            apply_inherited_metadata(
                &mut default_manifest,
                InheritedMetadataInput {
                    parent_timestamp: parent.timestamp,
                    proposal_timestamp: meta.proposal_timestamp,
                    fork_timestamp,
                    proposer: meta.proposer,
                    anchor_block_number: parent.anchor_block_number,
                    parent_block_number: parent.block_number,
                    parent_gas_limit: parent.gas_limit,
                    chain_id: meta.chain_id,
                },
            );
            Ok(default_manifest)
        }
    }
}

/// Return the total number of blocks contained in the derivation source.
#[must_use]
pub const fn block_count(manifest: &DerivationSourceManifest) -> usize {
    manifest.blocks.len()
}

fn decode_inline_manifest(
    data_source: Option<&InputDataSource>,
    offset: usize,
    max_blocks: usize,
) -> DerivationSourceManifest {
    let Some(data_source) = data_source else {
        return DerivationSourceManifest::default();
    };

    if !data_source.tx_data_from_calldata.is_empty() {
        return DerivationSourceManifest::decompress_and_decode_with_max_blocks(
            &data_source.tx_data_from_calldata,
            offset,
            max_blocks,
        )
        .unwrap_or_default();
    }

    if data_source.tx_data_from_blob.is_empty() {
        return DerivationSourceManifest::default();
    }

    let concatenated = data_source
        .tx_data_from_blob
        .iter()
        .flat_map(|chunk| chunk.iter().copied())
        .collect::<Vec<_>>();
    DerivationSourceManifest::decompress_and_decode_with_max_blocks(
        &concatenated,
        offset,
        max_blocks,
    )
    .unwrap_or_default()
}

fn decode_blob_backed_manifest(
    data_source: &InputDataSource,
    offset: usize,
    max_blocks: usize,
) -> Result<DerivationSourceManifest, SourceDerivationError> {
    if data_source.tx_data_from_blob.is_empty() {
        return Err(SourceDerivationError::MissingBlobData);
    }

    let blobs = data_source
        .tx_data_from_blob
        .iter()
        .enumerate()
        .map(|(index, raw)| {
            Blob::try_from(raw.as_slice()).map_err(|_| SourceDerivationError::InvalidBlobLength {
                index,
                actual: raw.len(),
                expected: BYTES_PER_BLOB,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let decoded =
        BlobCoder::decode_blobs(&blobs).ok_or(SourceDerivationError::InvalidBlobEncoding)?;
    let mut concatenated = Vec::new();
    for chunk in decoded {
        concatenated.extend(chunk);
    }

    Ok(
        DerivationSourceManifest::decompress_and_decode_with_max_blocks(
            &concatenated,
            offset,
            max_blocks,
        )
        .unwrap_or_default(),
    )
}

fn is_source_offset_valid(source: &DerivationSource) -> bool {
    source.blobSlice.offset.to::<usize>() <= MAX_MANIFEST_OFFSET
}

fn validate_timestamps(
    manifest: &DerivationSourceManifest,
    parent_timestamp: u64,
    proposal_timestamp: u64,
    fork_timestamp: u64,
    chain_id: u64,
) -> bool {
    let mut parent_ts = parent_timestamp;

    for block in &manifest.blocks {
        let lower_bound =
            compute_timestamp_lower_bound(parent_ts, proposal_timestamp, fork_timestamp, chain_id);
        if lower_bound > proposal_timestamp {
            return false;
        }

        if block.timestamp < lower_bound || block.timestamp > proposal_timestamp {
            return false;
        }

        parent_ts = block.timestamp;
    }

    true
}

fn compute_timestamp_lower_bound(
    parent_timestamp: u64,
    proposal_timestamp: u64,
    fork_timestamp: u64,
    chain_id: u64,
) -> u64 {
    let timestamp_max_offset = timestamp_max_offset_for_chain(chain_id);
    let lower_bound = parent_timestamp.saturating_add(1);
    let lower_bound = if proposal_timestamp > timestamp_max_offset {
        lower_bound.max(proposal_timestamp - timestamp_max_offset)
    } else {
        lower_bound
    };
    lower_bound.max(fork_timestamp)
}

fn validate_anchor_numbers(
    manifest: &DerivationSourceManifest,
    origin_block_number: u64,
    parent_anchor_block_number: u64,
    is_forced_inclusion: bool,
    chain_id: u64,
) -> bool {
    let mut parent_anchor = parent_anchor_block_number;
    let mut highest_anchor = parent_anchor_block_number;
    let max_anchor_offset = max_anchor_offset_for_chain(chain_id);

    for block in &manifest.blocks {
        let anchor = block.anchor_block_number;

        if anchor < parent_anchor || anchor > origin_block_number {
            return false;
        }

        if origin_block_number > max_anchor_offset
            && anchor < origin_block_number - max_anchor_offset
        {
            return false;
        }

        if anchor > highest_anchor {
            highest_anchor = anchor;
        }

        parent_anchor = anchor;
    }

    is_forced_inclusion || highest_anchor > parent_anchor_block_number
}

fn validate_gas_limit(
    manifest: &DerivationSourceManifest,
    parent_block_number: u64,
    parent_gas_limit: u64,
) -> bool {
    let mut effective_parent_gas_limit =
        effective_parent_gas_limit(parent_block_number, parent_gas_limit);

    for block in &manifest.blocks {
        let (lower_bound, upper_bound) = gas_limit_bounds(effective_parent_gas_limit);
        if block.gas_limit < lower_bound || block.gas_limit > upper_bound {
            return false;
        }

        effective_parent_gas_limit = block.gas_limit;
    }

    true
}

fn gas_limit_bounds(parent_gas_limit: u64) -> (u64, u64) {
    let parent = u128::from(parent_gas_limit);
    let denominator = u128::from(GAS_LIMIT_DENOMINATOR);
    let change = u128::from(BLOCK_GAS_LIMIT_MAX_CHANGE);
    let upper = parent.saturating_mul(denominator.saturating_add(change)) / denominator;
    let upper =
        u64::try_from(upper.min(u128::from(MAX_BLOCK_GAS_LIMIT))).expect("bounded gas limit");
    let lower = parent.saturating_mul(denominator.saturating_sub(change)) / denominator;
    let lower = u64::try_from(
        lower
            .max(u128::from(MIN_BLOCK_GAS_LIMIT))
            .min(u128::from(upper)),
    )
    .expect("bounded gas limit");

    (lower, upper)
}

const fn effective_parent_gas_limit(parent_block_number: u64, parent_gas_limit: u64) -> u64 {
    if parent_block_number == 0 {
        parent_gas_limit
    } else {
        parent_gas_limit.saturating_sub(ANCHOR_V3_V4_GAS_LIMIT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shasta::manifest::BlockManifest;

    fn block_manifest(anchor_block_number: u64) -> BlockManifest {
        BlockManifest {
            timestamp: 1_001,
            coinbase: Address::repeat_byte(0x11),
            anchor_block_number,
            gas_limit: 29_000_000,
            transactions: Vec::new(),
        }
    }

    fn proposal_metadata() -> ProposalMetadata {
        ProposalMetadata {
            proposal_timestamp: 1_100,
            origin_block_number: 1_000,
            proposer: Address::repeat_byte(0x22),
            chain_id: crate::shasta::constants::TAIKO_HOODI_CHAIN_ID,
        }
    }

    fn parent_context() -> ParentBlockContext {
        ParentBlockContext {
            timestamp: 1_000,
            gas_limit: 30_000_000,
            block_number: 10,
            anchor_block_number: 900,
        }
    }

    #[test]
    fn validate_source_manifest_marks_default() {
        let ctx = ValidationContext {
            parent_timestamp: 1_000,
            parent_gas_limit: 30_000_000,
            parent_block_number: 0,
            parent_anchor_block_number: 0,
            proposal_timestamp: 1_010,
            origin_block_number: 1_000,
            is_forced_inclusion: false,
            fork_timestamp: 0,
            chain_id: crate::shasta::constants::TAIKO_HOODI_CHAIN_ID,
        };

        let manifest = DerivationSourceManifest { blocks: Vec::new() };
        assert_eq!(
            validate_source_manifest(&manifest, &ctx),
            Err(ValidationError::EmptyManifest)
        );

        let manifest = DerivationSourceManifest {
            blocks: vec![BlockManifest {
                timestamp: ctx.parent_timestamp,
                coinbase: Address::ZERO,
                anchor_block_number: ctx.parent_anchor_block_number,
                gas_limit: 0,
                transactions: Vec::new(),
            }],
        };
        assert_eq!(
            validate_source_manifest(&manifest, &ctx),
            Err(ValidationError::DefaultManifest)
        );
    }

    #[test]
    fn apply_inherited_metadata_sets_fields() {
        let mut manifest = DerivationSourceManifest {
            blocks: vec![BlockManifest::default(), BlockManifest::default()],
        };
        apply_inherited_metadata(
            &mut manifest,
            InheritedMetadataInput {
                parent_timestamp: 1_000,
                proposal_timestamp: 2_000,
                fork_timestamp: 1_500,
                proposer: Address::repeat_byte(0x11),
                anchor_block_number: 900,
                parent_block_number: 10,
                parent_gas_limit: 30_000_000,
                chain_id: crate::shasta::constants::TAIKO_HOODI_CHAIN_ID,
            },
        );

        for block in &manifest.blocks {
            assert_eq!(block.coinbase, Address::repeat_byte(0x11));
            assert_eq!(block.anchor_block_number, 900);
            assert_eq!(block.gas_limit, 29_000_000);
        }
    }

    #[test]
    fn prepare_source_manifest_uses_inline_payloads() {
        let source = DerivationSource::default();
        let manifest = DerivationSourceManifest {
            blocks: vec![block_manifest(901)],
        };
        let data_source = InputDataSource {
            tx_data_from_calldata: manifest.encode_and_compress().expect("payload"),
            ..Default::default()
        };

        let prepared = prepare_source_manifest(
            &source,
            Some(&data_source),
            parent_context(),
            proposal_metadata(),
            0,
        )
        .expect("prepared manifest");

        assert!(prepared.blocks[0].transactions.is_empty());
        assert_eq!(prepared.blocks[0].anchor_block_number, 901);
    }

    #[test]
    fn prepare_source_manifest_accepts_unzen_block_limit() {
        let source = DerivationSource::default();
        let manifest = DerivationSourceManifest {
            blocks: (0..crate::shasta::constants::UNZEN_DERIVATION_SOURCE_MAX_BLOCKS)
                .map(|index| {
                    let mut block = block_manifest(901);
                    block.timestamp = 1_001 + u64::try_from(index).expect("fits u64");
                    block
                })
                .collect(),
        };
        let data_source = InputDataSource {
            tx_data_from_calldata: manifest.encode_and_compress().expect("payload"),
            ..Default::default()
        };
        let meta = ProposalMetadata {
            proposal_timestamp: 2_000,
            origin_block_number: 1_000,
            proposer: Address::repeat_byte(0x22),
            chain_id: crate::shasta::constants::TAIKO_DEVNET_CHAIN_ID,
        };

        let prepared =
            prepare_source_manifest(&source, Some(&data_source), parent_context(), meta, 0)
                .expect("prepared manifest");

        assert_eq!(
            prepared.blocks.len(),
            crate::shasta::constants::UNZEN_DERIVATION_SOURCE_MAX_BLOCKS
        );
    }

    #[test]
    fn prepare_source_manifest_rejects_malformed_blob_backed_payloads() {
        let mut source = DerivationSource::default();
        source.blobSlice.blobHashes = vec![alloy_primitives::B256::repeat_byte(0xAA)];
        let data_source = InputDataSource {
            tx_data_from_blob: vec![vec![0xAB; BYTES_PER_BLOB]],
            ..Default::default()
        };

        let err = prepare_source_manifest(
            &source,
            Some(&data_source),
            parent_context(),
            proposal_metadata(),
            0,
        )
        .expect_err("malformed blob payload should be rejected");

        assert_eq!(err, SourceDerivationError::InvalidBlobEncoding);
    }

    #[test]
    fn prepare_source_manifest_defaults_invalid_forced_inclusion_segments() {
        let source = DerivationSource {
            isForcedInclusion: true,
            ..Default::default()
        };
        let manifest = DerivationSourceManifest {
            blocks: vec![block_manifest(901), block_manifest(902)],
        };
        let data_source = InputDataSource {
            tx_data_from_calldata: manifest.encode_and_compress().expect("payload"),
            ..Default::default()
        };

        let prepared = prepare_source_manifest(
            &source,
            Some(&data_source),
            parent_context(),
            proposal_metadata(),
            0,
        )
        .expect("prepared manifest");

        assert_eq!(prepared.blocks.len(), 1);
        assert!(prepared.blocks[0].transactions.is_empty());
        assert_eq!(prepared.blocks[0].coinbase, proposal_metadata().proposer);
        assert_eq!(
            prepared.blocks[0].anchor_block_number,
            parent_context().anchor_block_number
        );
    }
}
