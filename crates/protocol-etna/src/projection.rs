//! Binding of a Comet header and its original envelope to the supplied execution projection.
//!
//! The caller independently authenticates the context. This module binds the
//! header and ordered transactions to the exact committed bytes and expected
//! context. The composing verifier must apply the existing strict EL verifier
//! to the matched header and transactions decoded from the returned payloads,
//! and strictly decode and authenticate the witness items.

use alethia_reth_primitives::EtnaExtraData;
use alloc::vec::Vec;
use alloy_consensus::Header as ExecutionHeader;
use alloy_eips::eip2718::Encodable2718;
use alloy_primitives::B256;
use alloy_rlp::Header as RlpHeader;
use etna_core::{
    comet::{
        APP_VERSION,
        verify::{data_hash, header_hash},
    },
    domain::{DeploymentDomain, DomainError, chain_id},
    params::UINT48_MAX,
};
use tendermint::Hash;

const ENVELOPE_VERSION: u8 = 6;
const MAX_EXECUTION_ITEM_BYTES: usize = 124_000;

/// Deployment and chain position independently authenticated by the caller.
#[derive(Clone, Copy, Debug)]
pub struct ProjectionContext {
    /// Authenticated deployment and recovery generation.
    pub domain: DeploymentDomain,
    /// Fixed, authenticated height immediately before the generation begins.
    pub origin_height: u64,
    /// Authenticated height expected for both headers.
    pub height: u64,
    /// Expected hash of the current Comet header.
    pub expected_comet_hash: B256,
    /// Expected parent Comet header hash at later heights. At the generation's
    /// first height this field is unused, and the old origin is authenticated separately.
    pub parent_comet_hash: B256,
    /// Expected execution parent hash.
    pub parent_el_hash: B256,
    /// Maximum bytes of the original Comet envelope.
    pub max_envelope_bytes: usize,
}

/// Exact execution byte and context binding for the committed envelope.
///
/// The composing verifier must establish the required Comet header, range and
/// certificate relations, authenticate the witnesses, verify forced inclusion
/// and data availability, and apply strict EL verification to the same matched
/// header and transactions derived from the committed payloads.
#[derive(Debug)]
pub struct ExecutionProjection<'a> {
    execution_item_bytes: &'a [u8],
    transaction_bytes: Vec<&'a [u8]>,
    execution_hash: B256,
    comet_hash: B256,
    anchor_number: u64,
    anchor_witness: Option<&'a [u8]>,
    forced_witness: Option<&'a [u8]>,
}

impl<'a> ExecutionProjection<'a> {
    /// Original encoded execution item, including its list prefix, for later DA equality.
    #[must_use]
    pub const fn execution_item_bytes(&self) -> &'a [u8] {
        self.execution_item_bytes
    }

    /// Original committed EIP-2718 payloads in execution order, without RLP string prefixes.
    ///
    /// Only the supplied transactions' EIP-2718 encodings are bound. Cached
    /// hashes and pre-recovered signers remain caller-supplied. Derive hashes
    /// for forced inclusion from these bytes and strictly decode and validate
    /// transactions and recover signers from them for EL verification.
    #[must_use]
    pub const fn transaction_bytes(&self) -> &[&'a [u8]] {
        self.transaction_bytes.as_slice()
    }

    /// Hash derived from the matched execution header.
    #[must_use]
    pub const fn execution_hash(&self) -> B256 {
        self.execution_hash
    }

    /// Hash derived from the matched Comet header.
    #[must_use]
    pub const fn comet_hash(&self) -> B256 {
        self.comet_hash
    }

    /// Unsigned 48-bit anchor number in the execution header's extra data.
    #[must_use]
    pub const fn anchor_number(&self) -> u64 {
        self.anchor_number
    }

    /// Raw inner anchor item, whose fields still require strict decoding and verification.
    #[must_use]
    pub const fn anchor_witness(&self) -> Option<&'a [u8]> {
        self.anchor_witness
    }

    /// Raw inner forced-inclusion item, whose fields still require strict decoding and verification.
    #[must_use]
    pub const fn forced_witness(&self) -> Option<&'a [u8]> {
        self.forced_witness
    }
}

/// The supplied data failed execution byte or context binding.
#[derive(Debug, thiserror::Error)]
pub enum ProjectionError {
    /// The Comet transaction list must contain a single envelope.
    #[error("expected one Comet transaction")]
    CometTransactionCount,
    /// The original envelope exceeds the caller's bound.
    #[error("Comet envelope exceeds the configured byte limit")]
    EnvelopeSize,
    /// The context height and origin do not describe a valid generation position.
    #[error("invalid Etna height or generation origin")]
    Height,
    /// The domain cannot be represented as an Etna chain ID.
    #[error("invalid Etna deployment domain: {0}")]
    Domain(#[from] DomainError),
    /// The Comet header has a different generation-specific chain ID.
    #[error("unexpected Etna Comet chain ID")]
    ChainId,
    /// The Comet header has an unsupported application version.
    #[error("unsupported Etna Comet application version")]
    AppVersion,
    /// The Comet header does not hash to the expected current identity.
    #[error("Comet header hash differs from expected hash")]
    CometHash,
    /// The Comet header does not carry the expected generation ancestry.
    #[error("unexpected Etna Comet parent link")]
    CometParent,
    /// The original envelope is not committed by the supplied Comet header.
    #[error("Comet data hash does not bind the envelope")]
    DataHash,
    /// The original envelope is missing its version or uses a different version.
    #[error("expected Etna envelope version 6")]
    EnvelopeVersion,
    /// An envelope boundary carries invalid RLP or the wrong kind of item.
    #[error("invalid Etna envelope RLP: {0}")]
    Rlp(#[from] alloy_rlp::Error),
    /// An envelope list has extra items or the outer list has trailing bytes.
    #[error("unexpected trailing Etna envelope data")]
    TrailingData,
    /// The full original execution item exceeds the protocol byte limit.
    #[error("original execution item exceeds the Etna byte limit")]
    ExecutionItemSize,
    /// The execution header has a different parent from the expected identity.
    #[error("execution parent differs from expected parent")]
    ExecutionParent,
    /// Execution extra data has a wrong length, base fee share, or generation.
    #[error("invalid Etna execution extra data")]
    ExtraData,
    /// The exact header item does not encode the supplied typed execution header.
    #[error("execution header does not match the envelope")]
    ExecutionHeader,
    /// The typed execution header does not round trip through its RLP representation.
    #[error("execution header does not round trip through RLP")]
    HeaderRoundTrip,
    /// Typed execution transactions differ from the committed ordered list.
    #[error("execution transactions do not match the envelope")]
    ExecutionTransactions,
}

/// Binds the supplied header and ordered transactions to the committed bytes
/// and independently authenticated caller context.
///
/// At the start of a generation, the Comet header must have no parent ID. At later
/// heights, this binding checks the parent header hash. The composing verifier
/// establishes the header, range and certificate relations required by the proof
/// specification, including complete `BlockID` relations where required. This
/// binding does not check evidence, part sets, or the embedded predecessor
/// commit. The Comet app hash is included in the header hash as opaque data.
///
/// The result establishes byte and context binding. The composing verifier
/// must run the existing strict EL verifier on the matched header and ordered
/// transactions decoded from [`ExecutionProjection::transaction_bytes`]. That
/// verifier checks the transaction root, ommers, withdrawals, fork rules and
/// state transition. The composing verifier must also strictly decode and
/// authenticate the returned witnesses, verify forced inclusion, and bind
/// data availability.
///
/// # Errors
///
/// Returns an error on a context mismatch, invalid boundary framing, a byte
/// limit violation, or a mismatch with the committed execution bytes.
pub fn verify_execution_projection<'a, T: Encodable2718>(
    comet_header: &tendermint::block::Header,
    comet_transactions: &'a [Vec<u8>],
    execution_header: &ExecutionHeader,
    execution_transactions: &[T],
    context: &ProjectionContext,
) -> Result<ExecutionProjection<'a>, ProjectionError> {
    let [envelope] = comet_transactions else {
        return Err(ProjectionError::CometTransactionCount);
    };
    if envelope.len() > context.max_envelope_bytes {
        return Err(ProjectionError::EnvelopeSize);
    }

    let comet_hash = verify_comet_context(comet_header, context)?;
    if comet_header.data_hash != Some(data_hash(comet_transactions)) {
        return Err(ProjectionError::DataHash);
    }

    let Some((&ENVELOPE_VERSION, mut body)) = envelope.split_first() else {
        return Err(ProjectionError::EnvelopeVersion);
    };
    let mut fields = RlpHeader::decode_bytes(&mut body, true)?;
    require_empty(body)?;
    let (execution_item_bytes, block_fields) = list_item(&mut fields)?;
    if execution_item_bytes.len() > MAX_EXECUTION_ITEM_BYTES {
        return Err(ProjectionError::ExecutionItemSize);
    }
    let anchor_witness = optional_witness(&mut fields)?;
    let forced_witness = optional_witness(&mut fields)?;
    require_empty(fields)?;

    let anchor_number = verify_execution_header(execution_header, context)?;
    let transaction_bytes =
        verify_execution_block(block_fields, execution_header, execution_transactions)?;

    Ok(ExecutionProjection {
        execution_item_bytes,
        transaction_bytes,
        execution_hash: execution_header.hash_slow(),
        comet_hash,
        anchor_number,
        anchor_witness,
        forced_witness,
    })
}

fn verify_comet_context(
    header: &tendermint::block::Header,
    context: &ProjectionContext,
) -> Result<B256, ProjectionError> {
    let first_height = context
        .origin_height
        .checked_add(1)
        .ok_or(ProjectionError::Height)?;
    if context.height < first_height
        || context.height > UINT48_MAX
        || header.height.value() != context.height
    {
        return Err(ProjectionError::Height);
    }
    let expected_chain_id = chain_id(&context.domain)?;
    if header.chain_id.as_str() != expected_chain_id {
        return Err(ProjectionError::ChainId);
    }
    if header.version.app != APP_VERSION {
        return Err(ProjectionError::AppVersion);
    }
    let Hash::Sha256(comet_hash) = header_hash(header) else {
        return Err(ProjectionError::CometHash);
    };
    let comet_hash = B256::from(comet_hash);
    if comet_hash != context.expected_comet_hash {
        return Err(ProjectionError::CometHash);
    }
    match header.last_block_id {
        None if context.height == first_height => Ok(comet_hash),
        Some(parent)
            if context.height > first_height
                && parent.hash.as_bytes() == context.parent_comet_hash.as_slice() =>
        {
            Ok(comet_hash)
        }
        _ => Err(ProjectionError::CometParent),
    }
}

fn verify_execution_header(
    header: &ExecutionHeader,
    context: &ProjectionContext,
) -> Result<u64, ProjectionError> {
    if header.number != context.height {
        return Err(ProjectionError::Height);
    }
    if header.parent_hash != context.parent_el_hash {
        return Err(ProjectionError::ExecutionParent);
    }

    decode_anchor_number(header.extra_data.as_ref(), context.domain.generation)
}

/// Checks the Etna fee share and generation and decodes the encoded anchor number.
pub(crate) fn decode_anchor_number(
    extra_data: &[u8],
    generation: u64,
) -> Result<u64, ProjectionError> {
    let fields = EtnaExtraData::decode(extra_data).ok_or(ProjectionError::ExtraData)?;
    if fields.generation() != generation {
        return Err(ProjectionError::ExtraData);
    }
    // Every bitmap bit remains committed by the header/envelope/DA equality.
    // Committee and missed-signature semantics require independent authority.
    Ok(fields.anchor_block_number())
}

fn verify_execution_block<'a, T: Encodable2718>(
    mut fields: &'a [u8],
    header: &ExecutionHeader,
    transactions: &[T],
) -> Result<Vec<&'a [u8]>, ProjectionError> {
    let (header_item, _) = list_item(&mut fields)?;
    let mut tx_items = RlpHeader::decode_bytes(&mut fields, true)?;
    require_empty(fields)?;

    let encoded_header = alloy_rlp::encode(header);
    if header_item != encoded_header {
        return Err(ProjectionError::ExecutionHeader);
    }
    verify_execution_header_round_trip(header, &encoded_header)?;

    let mut transaction_bytes = Vec::new();
    for tx in transactions {
        if tx_items.is_empty() {
            return Err(ProjectionError::ExecutionTransactions);
        }
        let committed = RlpHeader::decode_bytes(&mut tx_items, false)?;
        // The original execution item's bound also bounds every string payload.
        // Compare the proposed encoded length before allocating the encoding.
        if tx.encode_2718_len() != committed.len() || tx.encoded_2718() != committed {
            return Err(ProjectionError::ExecutionTransactions);
        }
        transaction_bytes.push(committed);
    }
    if !tx_items.is_empty() {
        return Err(ProjectionError::ExecutionTransactions);
    }
    Ok(transaction_bytes)
}

/// Checks that the typed header fields are represented faithfully in its RLP encoding.
pub(crate) fn verify_execution_header_round_trip(
    header: &ExecutionHeader,
    encoded: &[u8],
) -> Result<(), ProjectionError> {
    let decoded: ExecutionHeader =
        alloy_rlp::decode_exact(encoded).map_err(|_| ProjectionError::HeaderRoundTrip)?;
    if &decoded != header {
        return Err(ProjectionError::HeaderRoundTrip);
    }
    Ok(())
}

fn list_item<'a>(bytes: &mut &'a [u8]) -> Result<(&'a [u8], &'a [u8]), ProjectionError> {
    let original = *bytes;
    let payload = RlpHeader::decode_bytes(bytes, true)?;
    Ok((&original[..original.len() - bytes.len()], payload))
}

fn optional_witness<'a>(bytes: &mut &'a [u8]) -> Result<Option<&'a [u8]>, ProjectionError> {
    let mut wrapper = RlpHeader::decode_bytes(bytes, true)?;
    if wrapper.is_empty() {
        return Ok(None);
    }
    let (item, _) = list_item(&mut wrapper)?;
    require_empty(wrapper)?;
    Ok(Some(item))
}

const fn require_empty(bytes: &[u8]) -> Result<(), ProjectionError> {
    if bytes.is_empty() {
        Ok(())
    } else {
        Err(ProjectionError::TrailingData)
    }
}
