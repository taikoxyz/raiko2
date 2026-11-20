//! Etna byte and identity projection for a complete range `(p, h]`.
//!
//! The caller authenticates the deployment, generation origin and range
//! context. The projection links the Comet and execution headers and preserves
//! the original envelope spans, including any forced-inclusion witness.
//! [`verify_range_statement_projection`] also binds the landing statement and
//! its interior cuts to the range.
//! The composing verifier still checks nested witnesses, the committee
//! schedule, certificates, strict EL execution and state transitions,
//! timestamps and anchors, forced inclusion, DA and blobs, and proof
//! output framing.

use alloc::vec::Vec;
use alloy_consensus::Header as ExecutionHeader;
use alloy_eips::eip2718::Encodable2718;
use alloy_primitives::{B256, U256};
use etna_core::{comet::verify::header_hash, domain::DeploymentDomain, schedule::UINT48_MAX};
use tendermint::{Hash, block::Header as CometHeader};

use crate::{
    projection::{
        ExecutionProjection, ProjectionContext, ProjectionError, verify_execution_projection,
    },
    statement::{LandStatement, LandStatementError, cut_leaf},
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

/// The supplied Etna landing statement could not be bound to the range.
#[derive(Debug, thiserror::Error)]
pub enum RangeStatementProjectionError {
    /// The supplied range failed byte or identity projection.
    #[error("invalid Etna landing statement range: {0}")]
    Range(#[from] RangeProjectionError),
    /// The supplied statement failed its canonical range or cut-count check.
    #[error("invalid Etna landing statement shape: {0}")]
    Statement(#[from] LandStatementError),
    /// A statement field disagreed with the matched range.
    #[error("Etna landing statement field {field} differs from the range")]
    StatementMismatch {
        /// Name of the mismatched field.
        field: &'static str,
    },
    /// There was not one post-block FI cursor per covered block.
    #[error("Etna landing statement requires {expected} post-block cursors, got {actual}")]
    CursorCount {
        /// Number of blocks in the range.
        expected: usize,
        /// Number of supplied cursors.
        actual: usize,
    },
    /// The FI cursor at the proof origin exceeds uint48.
    #[error("Etna landing statement origin cursor exceeds uint48")]
    OriginCursor,
    /// A post-block FI cursor exceeds uint48.
    #[error("Etna post-block cursor exceeds uint48 at height {height}")]
    CursorBounds {
        /// Height of the block.
        height: u64,
    },
    /// A post-block FI cursor did not stay the same or increase by one.
    #[error("invalid Etna post-block cursor step at height {height}: {previous} to {actual}")]
    CursorStep {
        /// Height of the block.
        height: u64,
        /// Cursor before the block.
        previous: u64,
        /// Cursor after the block.
        actual: u64,
    },
    /// The terminal FI cursor differs from the statement.
    #[error("Etna landing statement terminal cursor differs from the supplied cursor")]
    TerminalCursor,
    /// The supplied cut leaf disagrees with the matched block and its cursor.
    #[error("Etna landing statement cut leaf mismatch at height {height}")]
    CutLeaf {
        /// Height of the interior block.
        height: u64,
    },
}

/// Returns the ordered, borrowed range projections and the landing statement
/// digest after binding the statement to the supplied range.
///
/// The proof origin can be a historical block within the generation. The
/// caller authenticates the deployment, proof origin and range context.
/// Each post-block FI cursor is checked for its uint48 width and for a valid
/// step relative to the previous cursor. The composing verifier authenticates
/// the origin cursor and checks each supplied cursor against actual FI behavior.
/// It also authenticates the L1 reference, registry, configuration, committee
/// selection and blob commitments and verifies certificates, nested witnesses,
/// strict execution, DA equality and proof framing.
///
/// # Errors
/// Returns an error for an invalid range, mismatched statement field or
/// interior cut, or an invalid cursor count, bound or transition.
pub fn verify_range_statement_projection<'a, T: Encodable2718>(
    blocks: &[RangeBlock<'a, T>],
    context: &RangeProjectionContext,
    statement: &LandStatement<'_>,
    post_block_cursors: &[u64],
) -> Result<(Vec<ExecutionProjection<'a>>, B256), RangeStatementProjectionError> {
    validate_range_block_count(context, blocks.len())?;
    let input = &statement.input;
    for (field, matches) in [
        (
            "l1_chain_id",
            statement.l1_chain_id == context.domain.l1_chain_id,
        ),
        (
            "l2_chain_id",
            statement.l2_chain_id == context.domain.l2_chain_id,
        ),
        ("inbox", statement.inbox == context.domain.inbox),
        (
            "generation",
            statement.generation == U256::from(context.domain.generation),
        ),
        (
            "parent_height",
            input.parent_height == U256::from(context.parent_height),
        ),
        (
            "last_height",
            input.last_height == U256::from(context.last_height),
        ),
        (
            "parent_hash",
            statement.parent_hash == context.parent_el_hash,
        ),
        (
            "parent_comet_hash",
            statement.parent_comet_hash == context.parent_comet_hash,
        ),
        (
            "last_block_hash",
            input.last_block_hash == context.last_el_hash,
        ),
        (
            "last_comet_hash",
            input.last_comet_hash == context.last_comet_hash,
        ),
    ] {
        if !matches {
            return Err(RangeStatementProjectionError::StatementMismatch { field });
        }
    }
    statement.validate_shape()?;
    validate_cursors(context, statement, post_block_cursors, blocks.len())?;

    let projections = verify_range_projection(blocks, context)?;
    let terminal = blocks.last().ok_or(RangeProjectionError::InvalidRange)?;
    if input.last_state_root != terminal.execution_header.state_root {
        return Err(RangeStatementProjectionError::StatementMismatch {
            field: "last_state_root",
        });
    }
    for ((projection, block), (&cursor, &leaf)) in projections
        .iter()
        .zip(blocks)
        .zip(post_block_cursors.iter().zip(statement.cut_leaves))
    {
        let height = block.execution_header.number;
        if cut_leaf(
            U256::from(height),
            projection.execution_hash(),
            projection.comet_hash(),
            U256::from(cursor),
        ) != leaf
        {
            return Err(RangeStatementProjectionError::CutLeaf { height });
        }
    }
    Ok((projections, statement.try_digest()?))
}

fn validate_cursors(
    context: &RangeProjectionContext,
    statement: &LandStatement<'_>,
    post_block_cursors: &[u64],
    block_count: usize,
) -> Result<(), RangeStatementProjectionError> {
    if post_block_cursors.len() != block_count {
        return Err(RangeStatementProjectionError::CursorCount {
            expected: block_count,
            actual: post_block_cursors.len(),
        });
    }
    let mut previous = u64::try_from(statement.forced_inclusion_head_before)
        .ok()
        .filter(|&cursor| cursor <= UINT48_MAX)
        .ok_or(RangeStatementProjectionError::OriginCursor)?;
    for (height, &actual) in
        (context.parent_height + 1..=context.last_height).zip(post_block_cursors)
    {
        if actual > UINT48_MAX {
            return Err(RangeStatementProjectionError::CursorBounds { height });
        }
        if !(previous..=previous + 1).contains(&actual) {
            return Err(RangeStatementProjectionError::CursorStep {
                height,
                previous,
                actual,
            });
        }
        previous = actual;
    }
    if U256::from(previous) != statement.input.forced_inclusion_head {
        return Err(RangeStatementProjectionError::TerminalCursor);
    }
    Ok(())
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
    validate_range_block_count(context, blocks.len())?;
    let RangeProjectionContext {
        domain,
        origin_height,
        parent_height,
        last_height,
        mut parent_el_hash,
        mut parent_comet_hash,
        last_el_hash,
        last_comet_hash,
        max_envelope_bytes,
        ..
    } = *context;

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
        parent_comet_hash = projection.comet_hash();
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

fn validate_range_block_count(
    context: &RangeProjectionContext,
    supplied_count: usize,
) -> Result<(), RangeProjectionError> {
    let RangeProjectionContext {
        origin_height,
        parent_height,
        last_height,
        max_blocks,
        ..
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
    if supplied_count != count {
        return Err(RangeProjectionError::BlockCount {
            expected: count,
            actual: supplied_count,
        });
    }
    Ok(())
}
