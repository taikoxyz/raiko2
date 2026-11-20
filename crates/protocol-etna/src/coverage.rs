//! Validator schedule binding and commit requirements for an Etna landing range.
//!
//! The caller must independently derive and authenticate the range endpoints
//! and the validator schedule, including the successor set at the terminal
//! height. This module checks the resulting validator commitments. The
//! composing verifier authenticates header identity and ancestry and verifies
//! the commit at each required height against its authorized active set and
//! deployment domain.

use alloc::vec::Vec;
use etna_core::{comet::validators::Validators, schedule::UINT48_MAX};
use tendermint::block::Header;

/// Caller-authenticated endpoints and resource limit for a complete landing range.
#[derive(Clone, Copy, Debug)]
pub struct CoverageContext {
    /// Height of the authenticated parent excluded from the range.
    pub parent_height: u64,
    /// Terminal height of the landing range.
    pub last_height: u64,
    /// Caller resource limit on the number of blocks in this range.
    pub max_blocks: usize,
}

/// Validator sets independently derived and authorized by the caller.
#[derive(Clone, Copy, Debug)]
pub struct ExpectedValidatorSets<'a> {
    /// Validator set active at the header's height.
    pub active: &'a Validators,
    /// Validator set active at the successor's height.
    pub next: &'a Validators,
}

/// Heights that require commit verification for the complete landing range.
#[derive(Debug)]
pub struct CommitRequirements {
    heights: Vec<u64>,
}

impl CommitRequirements {
    /// Required commit heights in ascending order.
    #[must_use]
    pub const fn required_commit_heights(&self) -> &[u64] {
        self.heights.as_slice()
    }
}

/// An invalid range or a mismatch with the expected validator schedule.
#[derive(Debug, thiserror::Error)]
pub enum CoverageError {
    /// The endpoints do not form a nonempty landing range within the uint48 bound.
    #[error("invalid landing range endpoints")]
    InvalidRange,
    /// The complete range exceeds the caller's block budget.
    #[error("landing range has {actual} blocks, exceeding limit {max}")]
    BlockLimit {
        /// Block count required by the endpoints.
        actual: u64,
        /// Maximum allowed block count.
        max: usize,
    },
    /// The header count disagrees with the complete range.
    #[error("landing range requires {expected} headers, got {actual}")]
    HeaderCount {
        /// Header count required by the endpoints.
        expected: usize,
        /// Supplied number of headers.
        actual: usize,
    },
    /// The expected schedule has the wrong number of entries.
    #[error("landing range requires {expected} validator schedule entries, got {actual}")]
    ScheduleCount {
        /// Entry count required by the endpoints.
        expected: usize,
        /// Supplied number of entries.
        actual: usize,
    },
    /// A header's height disagrees with its position in the range.
    #[error("expected Comet header at height {expected}, got {actual}")]
    HeaderHeight {
        /// Height required at this position.
        expected: u64,
        /// Height of the supplied header.
        actual: u64,
    },
    /// The current validator hash disagrees with the expected active set.
    #[error("current validator hash mismatch at height {height}")]
    ActiveHash {
        /// Height of the mismatching header.
        height: u64,
    },
    /// The next validator hash disagrees with the expected successor set.
    #[error("next validator hash mismatch at height {height}")]
    NextHash {
        /// Height of the mismatching header.
        height: u64,
    },
    /// The previous entry's successor set disagrees with this entry's active set.
    #[error("validator schedule does not link at height {height}")]
    ScheduleLink {
        /// Height where the previous and current schedule entries disagree.
        height: u64,
    },
}

/// Binds the current and next validator commitments at every height in the
/// complete landing range and derives the required commit heights.
///
/// The terminal height always requires a commit. An interior height requires
/// a commit when the canonical commitment to signing keys and assigned voting
/// powers changes at the next height. Pass the endpoints of the whole landing
/// range, since a child-proof cut need not require a commit.
///
/// # Errors
/// Returns an error for invalid endpoints, an exceeded block budget, incorrect
/// input counts or header heights, mismatched hashes, or inconsistent schedule
/// entries.
pub fn bind_committee_schedule(
    headers: &[Header],
    expected: &[ExpectedValidatorSets<'_>],
    context: &CoverageContext,
) -> Result<CommitRequirements, CoverageError> {
    let CoverageContext {
        parent_height,
        last_height,
        max_blocks,
    } = *context;
    let block_count = last_height
        .checked_sub(parent_height)
        .filter(|&count| count != 0 && last_height <= UINT48_MAX)
        .ok_or(CoverageError::InvalidRange)?;
    let first_height = parent_height
        .checked_add(1)
        .ok_or(CoverageError::InvalidRange)?;
    let count = usize::try_from(block_count)
        .ok()
        .filter(|&count| count <= max_blocks)
        .ok_or(CoverageError::BlockLimit {
            actual: block_count,
            max: max_blocks,
        })?;
    if headers.len() != count {
        return Err(CoverageError::HeaderCount {
            expected: count,
            actual: headers.len(),
        });
    }
    if expected.len() != count {
        return Err(CoverageError::ScheduleCount {
            expected: count,
            actual: expected.len(),
        });
    }

    let mut heights = Vec::new();
    let mut previous_next = None;
    for ((height, header), sets) in (first_height..=last_height).zip(headers).zip(expected) {
        if header.height.value() != height {
            return Err(CoverageError::HeaderHeight {
                expected: height,
                actual: header.height.value(),
            });
        }
        let active = sets.active.hash();
        let next = sets.next.hash();
        if header.validators_hash != active {
            return Err(CoverageError::ActiveHash { height });
        }
        if header.next_validators_hash != next {
            return Err(CoverageError::NextHash { height });
        }
        if previous_next.is_some_and(|previous| previous != active) {
            return Err(CoverageError::ScheduleLink { height });
        }
        if height < last_height && active != next {
            heights.push(height);
        }
        previous_next = Some(next);
    }
    heights.push(last_height);
    Ok(CommitRequirements { heights })
}
