//! Etna byte and identity projection for a complete range `(p, h]`.
//!
//! The caller authenticates the deployment, generation origin and range
//! context. The projection links the Comet and execution headers and preserves
//! the original envelope spans, including any forced-inclusion witness.
//! The composing verifier still checks nested witnesses, the committee
//! schedule, certificates, strict EL execution and state transitions,
//! timestamps and anchors, forced inclusion, DA and blobs, cuts, and proof
//! output framing.

use alloc::vec::Vec;
use alloy_consensus::Header as ExecutionHeader;
use alloy_eips::eip2718::Encodable2718;
use alloy_primitives::B256;
use etna_core::{comet::verify::header_hash, domain::DeploymentDomain, schedule::UINT48_MAX};
use tendermint::{Hash, block::Header as CometHeader};

use crate::projection::{
    ExecutionProjection, ProjectionContext, ProjectionError, verify_execution_projection,
};

/// The two supplied headers and their transaction lists at one height.
pub struct RangeBlock<'a, T> {
    /// Supplied Comet header.
    pub comet_header: &'a CometHeader,
    /// Original Comet transaction vector.
    pub comet_transactions: &'a [Vec<u8>],
    /// Supplied execution header.
    pub execution_header: &'a ExecutionHeader,
    /// Supplied execution transactions in committed order.
    pub execution_transactions: &'a [T],
}

/// Inputs the caller authenticates, and limits the caller sets for this range.
#[derive(Clone, Copy, Debug)]
pub struct RangeProjectionContext {
    /// Authenticated deployment and generation.
    pub domain: DeploymentDomain,
    /// Fixed height immediately before the current generation begins.
    pub origin_height: u64,
    /// Height of the excluded parent.
    pub parent_height: u64,
    /// Terminal height of the range.
    pub last_height: u64,
    /// Expected execution hash of the excluded parent.
    pub parent_el_hash: B256,
    /// Expected Comet hash of the excluded parent. At the generation origin,
    /// the caller authenticates this identity separately from Comet ancestry.
    pub parent_comet_hash: B256,
    /// Expected terminal execution hash.
    pub last_el_hash: B256,
    /// Expected terminal Comet hash.
    pub last_comet_hash: B256,
    /// Maximum blocks in the supplied range.
    pub max_blocks: usize,
    /// Maximum bytes per original envelope.
    pub max_envelope_bytes: usize,
}

/// The supplied range failed byte or identity projection.
#[derive(Debug, thiserror::Error)]
pub enum RangeProjectionError {
    /// The endpoints and origin do not describe a nonempty uint48 range
    /// within the current generation.
    #[error("invalid Etna projection range endpoints or generation origin")]
    InvalidRange,
    /// The complete range exceeds the caller's block budget.
    #[error("Etna projection range has {actual} blocks, exceeding limit {max}")]
    BlockLimit {
        /// Block count required by the endpoints.
        actual: u64,
        /// Maximum allowed block count.
        max: usize,
    },
    /// The supplied block count disagrees with the complete range.
    #[error("Etna projection range requires {expected} blocks, got {actual}")]
    BlockCount {
        /// Block count required by the endpoints.
        expected: usize,
        /// Supplied block count.
        actual: usize,
    },
    /// The supplied block failed byte or identity projection at this height.
    #[error("invalid Etna execution projection at height {height}: {source}")]
    Projection {
        /// Expected height of the failing block.
        height: u64,
        /// Failure from the per-block projection.
        #[source]
        source: ProjectionError,
    },
    /// The terminal Comet header hash differs from the expected endpoint.
    #[error("terminal Etna Comet header hash differs from expected hash")]
    TerminalCometHash,
    /// The terminal execution header hash differs from the expected endpoint.
    #[error("terminal Etna execution header hash differs from expected hash")]
    TerminalExecutionHash,
}

/// Binds every supplied block to its original envelope and consecutive range
/// position, and returns the borrowed projections in height order.
///
/// The first block of a generation has an empty Comet parent ID. After that,
/// ancestry compares predecessor header hashes. Each block inherits the
/// existing per-block `APP_VERSION` check, pending P01 policy alignment.
/// The caller enforces outer input framing and budgets before decoding or
/// allocating inputs, and uses the returned spans for the remaining checks
/// described in the module documentation.
///
/// # Errors
/// Returns an error for invalid endpoints, excess or incorrect block count,
/// failed per-block projection, or a mismatched terminal identity.
pub fn verify_range_projection<'a, T: Encodable2718>(
    blocks: &[RangeBlock<'a, T>],
    context: &RangeProjectionContext,
) -> Result<Vec<ExecutionProjection<'a>>, RangeProjectionError> {
    let RangeProjectionContext {
        domain,
        origin_height,
        parent_height,
        last_height,
        mut parent_el_hash,
        mut parent_comet_hash,
        last_el_hash,
        last_comet_hash,
        max_blocks,
        max_envelope_bytes,
    } = *context;
    if origin_height > parent_height || parent_height >= last_height || last_height > UINT48_MAX {
        return Err(RangeProjectionError::InvalidRange);
    }
    let block_count = last_height - parent_height;
    let count = usize::try_from(block_count)
        .ok()
        .filter(|&count| count <= max_blocks)
        .ok_or(RangeProjectionError::BlockLimit {
            actual: block_count,
            max: max_blocks,
        })?;
    if blocks.len() != count {
        return Err(RangeProjectionError::BlockCount {
            expected: count,
            actual: blocks.len(),
        });
    }

    let mut projections = Vec::new();
    for (height, block) in (parent_height + 1..=last_height).zip(blocks) {
        let Hash::Sha256(comet_hash) = header_hash(block.comet_header) else {
            return Err(RangeProjectionError::Projection {
                height,
                source: ProjectionError::CometHash,
            });
        };
        let comet_hash = B256::from(comet_hash);
        let projection = verify_execution_projection(
            block.comet_header,
            block.comet_transactions,
            block.execution_header,
            block.execution_transactions,
            &ProjectionContext {
                domain,
                origin_height,
                height,
                expected_comet_hash: comet_hash,
                parent_comet_hash,
                parent_el_hash,
                max_envelope_bytes,
            },
        )
        .map_err(|source| RangeProjectionError::Projection { height, source })?;
        parent_comet_hash = comet_hash;
        parent_el_hash = projection.execution_hash();
        projections.push(projection);
    }

    if parent_comet_hash != last_comet_hash {
        return Err(RangeProjectionError::TerminalCometHash);
    }
    if parent_el_hash != last_el_hash {
        return Err(RangeProjectionError::TerminalExecutionHash);
    }
    Ok(projections)
}
