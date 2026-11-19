//! Exact L1 header decoding and ancestry relative to an authenticated reference.

use alloc::vec::Vec;

use alloy_primitives::{B256, Bytes, keccak256};
use alloy_rlp::{Decodable, Header, PayloadView};

/// Maximum byte length of an L1 header's original RLP, including its list framing.
pub const MAX_L1_HEADER_BYTES: usize = 4096;

const PARENT_HASH_INDEX: usize = 0;
const STATE_ROOT_INDEX: usize = 3;
const NUMBER_INDEX: usize = 8;
const TIMESTAMP_INDEX: usize = 11;
const MIN_FIELDS: usize = TIMESTAMP_INDEX + 1;

/// Invalid original L1 header bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum L1HeaderError {
    /// The raw header exceeds the protocol byte bound.
    #[error("L1 header is {0} bytes, exceeding {MAX_L1_HEADER_BYTES}")]
    Oversize(usize),
    /// The outer list or a top-level item has invalid RLP framing.
    #[error("invalid L1 header RLP: {0}")]
    Rlp(alloy_rlp::Error),
    /// Extra bytes follow the outer list.
    #[error("{0} trailing bytes after the L1 header list")]
    TrailingBytes(usize),
    /// The list has fewer than the required number of fields.
    #[error("L1 header has {0} fields, at least {MIN_FIELDS} expected")]
    TooFewFields(usize),
    /// A positional field failed to decode as the required type.
    #[error("invalid L1 header field at position {index}: {error}")]
    Field {
        /// Position in the header's RLP list.
        index: usize,
        /// Reason the positional field could not be decoded.
        error: alloy_rlp::Error,
    },
}

/// Original L1 header bytes and the four fields Etna reads from them.
#[derive(Debug)]
pub struct L1HeaderRef<'a> {
    raw: &'a [u8],
    hash: B256,
    parent_hash: B256,
    state_root: B256,
    number: u64,
    timestamp: u64,
}

impl<'a> L1HeaderRef<'a> {
    /// Checks canonical top-level RLP framing and hashes the original bounded header.
    ///
    /// Unused items are checked for top-level RLP framing. Their contents,
    /// including the payloads of any lists, are opaque to this decoder.
    ///
    /// # Errors
    ///
    /// Returns an error if the header exceeds the byte bound, has invalid
    /// framing or trailing bytes, or lacks any of the required typed fields.
    pub fn decode(raw: &'a [u8]) -> Result<Self, L1HeaderError> {
        check_header_size(raw.len())?;

        let mut remaining = raw;
        let fields = match Header::decode_raw(&mut remaining).map_err(L1HeaderError::Rlp)? {
            PayloadView::List(fields) => fields,
            PayloadView::String(_) => {
                return Err(L1HeaderError::Rlp(alloy_rlp::Error::UnexpectedString));
            }
        };
        if !remaining.is_empty() {
            return Err(L1HeaderError::TrailingBytes(remaining.len()));
        }
        if fields.len() < MIN_FIELDS {
            return Err(L1HeaderError::TooFewFields(fields.len()));
        }

        let parent_hash = decode_field(&fields, PARENT_HASH_INDEX)?;
        let state_root = decode_field(&fields, STATE_ROOT_INDEX)?;
        let number = decode_field(&fields, NUMBER_INDEX)?;
        let timestamp = decode_field(&fields, TIMESTAMP_INDEX)?;
        Ok(Self {
            raw,
            hash: keccak256(raw),
            parent_hash,
            state_root,
            number,
            timestamp,
        })
    }

    /// Exact RLP bytes as supplied by the caller.
    #[must_use]
    pub const fn raw(&self) -> &'a [u8] {
        self.raw
    }

    /// Keccak-256 of the exact raw bytes.
    #[must_use]
    pub const fn hash(&self) -> B256 {
        self.hash
    }

    /// Hash of the parent L1 block.
    #[must_use]
    pub const fn parent_hash(&self) -> B256 {
        self.parent_hash
    }

    /// State root committed by the header.
    #[must_use]
    pub const fn state_root(&self) -> B256 {
        self.state_root
    }

    /// L1 block number.
    #[must_use]
    pub const fn number(&self) -> u64 {
        self.number
    }

    /// L1 block timestamp in seconds since the Unix epoch.
    #[must_use]
    pub const fn timestamp(&self) -> u64 {
        self.timestamp
    }
}

const fn check_header_size(len: usize) -> Result<(), L1HeaderError> {
    if len > MAX_L1_HEADER_BYTES {
        return Err(L1HeaderError::Oversize(len));
    }
    Ok(())
}

fn decode_field<T: Decodable>(fields: &[&[u8]], index: usize) -> Result<T, L1HeaderError> {
    alloy_rlp::decode_exact(fields[index]).map_err(|error| L1HeaderError::Field { index, error })
}

/// Authenticated L1 reference identity and caller-defined witness resource limits.
///
/// The caller must bind the reference hash and number to the landing statement
/// and the L1 contract's canonical reference check before relying on any roots
/// from this context. Resource limits bound verification work and memory.
#[derive(Debug, Clone, Copy)]
pub struct L1AncestryContext {
    /// Independently authenticated raw reference header hash.
    pub reference_hash: B256,
    /// Independently authenticated L1 reference block number.
    pub reference_number: u64,
    /// Maximum count of supplied L1 headers, including the reference.
    pub max_headers: usize,
    /// Maximum total bytes of the supplied raw L1 header lists.
    pub max_total_bytes: usize,
}

/// The witness exceeds resource limits or fails to establish contiguous L1 ancestry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum L1AncestryError {
    /// A witness must include its reference header.
    #[error("L1 ancestry is missing the reference header")]
    Empty,
    /// The number of supplied headers exceeds the caller's limit.
    #[error("L1 ancestry has {actual} headers, exceeding limit {max}")]
    CountLimit {
        /// Number of supplied headers.
        actual: usize,
        /// Maximum allowed headers.
        max: usize,
    },
    /// The cumulative byte length exceeds the caller's limit or cannot fit in a `usize`.
    #[error("L1 ancestry exceeds the total byte limit of {max}")]
    TotalBytesLimit {
        /// Maximum allowed total raw header bytes.
        max: usize,
    },
    /// One of the supplied raw headers cannot be decoded.
    #[error("invalid L1 ancestry header at index {index}: {error}")]
    Header {
        /// Index of the failed header in newest-first order.
        index: usize,
        /// Header decoding failure.
        #[source]
        error: L1HeaderError,
    },
    /// The raw hash differs from the reference or newer header's parent hash.
    #[error("L1 ancestry header at index {index} has hash {actual}, expected {expected}")]
    HashMismatch {
        /// Index of the failed header in newest-first order.
        index: usize,
        /// Expected reference or parent hash.
        expected: B256,
        /// Hash computed from the supplied raw bytes.
        actual: B256,
    },
    /// The decoded number differs from the reference or immediate parent number.
    #[error("L1 ancestry header at index {index} has number {actual}, expected {expected}")]
    NumberMismatch {
        /// Index of the failed header in newest-first order.
        index: usize,
        /// Expected reference or parent number.
        expected: u64,
        /// Number decoded from the supplied raw bytes.
        actual: u64,
    },
    /// A witness contains another header after block zero.
    #[error("L1 ancestry header at index {index} would precede block zero")]
    BeforeGenesis {
        /// Index of the header following block zero in the witness.
        index: usize,
    },
}

/// Headers linked to the supplied reference by raw parent hashes and contiguous numbers.
#[derive(Debug)]
pub struct L1Ancestry<'a> {
    headers: Vec<L1HeaderRef<'a>>,
}

impl<'a> L1Ancestry<'a> {
    /// Verified headers in newest-first order.
    #[must_use]
    pub const fn headers(&self) -> &[L1HeaderRef<'a>] {
        self.headers.as_slice()
    }

    /// Returns the verified header at this block number if covered by the witness.
    #[must_use]
    pub fn header(&self, number: u64) -> Option<&L1HeaderRef<'a>> {
        let reference = self.headers.first()?;
        let offset = usize::try_from(reference.number().checked_sub(number)?).ok()?;
        self.headers.get(offset)
    }
}

/// Verifies a reference header and any contiguous prefix of its ancestors.
///
/// Supply raw headers newest first. Verification establishes only the ancestry
/// relative to the authenticated reference in [`L1AncestryContext`]. The
/// composing verifier establishes reference provenance and L1 finality where
/// required, checks anchor selection, age and progression, binds the execution
/// header's anchor fields, and authenticates the account, storage and committee
/// witnesses against the appropriate roots. It also verifies consensus,
/// execution, forced inclusion, data availability and the full landing proof.
///
/// # Errors
///
/// Returns an error if the input is empty or exceeds a budget, a header is
/// malformed, or a required number or raw hash comparison fails.
pub fn verify_l1_ancestry<'a>(
    raw_headers: &'a [Bytes],
    context: &L1AncestryContext,
) -> Result<L1Ancestry<'a>, L1AncestryError> {
    let count = raw_headers.len();
    if count == 0 {
        return Err(L1AncestryError::Empty);
    }
    if count > context.max_headers {
        return Err(L1AncestryError::CountLimit {
            actual: count,
            max: context.max_headers,
        });
    }

    let mut remaining_bytes = context.max_total_bytes;
    for (index, raw) in raw_headers.iter().enumerate() {
        check_header_size(raw.len()).map_err(|error| L1AncestryError::Header { index, error })?;
        remaining_bytes =
            remaining_bytes
                .checked_sub(raw.len())
                .ok_or(L1AncestryError::TotalBytesLimit {
                    max: context.max_total_bytes,
                })?;
    }

    let mut headers = Vec::with_capacity(count);
    let mut expected_hash = context.reference_hash;
    let mut expected_number = Some(context.reference_number);
    for (index, raw) in raw_headers.iter().enumerate() {
        let number = expected_number.ok_or(L1AncestryError::BeforeGenesis { index })?;
        let header =
            L1HeaderRef::decode(raw).map_err(|error| L1AncestryError::Header { index, error })?;
        if header.hash() != expected_hash {
            return Err(L1AncestryError::HashMismatch {
                index,
                expected: expected_hash,
                actual: header.hash(),
            });
        }
        if header.number() != number {
            return Err(L1AncestryError::NumberMismatch {
                index,
                expected: number,
                actual: header.number(),
            });
        }
        expected_hash = header.parent_hash();
        expected_number = number.checked_sub(1);
        headers.push(header);
    }

    Ok(L1Ancestry { headers })
}
