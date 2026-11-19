//! Etna consensus input checks and landing statement encoding.
//!
//! The per-block certified-envelope gate implements the earlier relation.
//! It checks the bare execution parent hash as the `CometBFT` app hash and
//! verifies the embedded predecessor commit after the first block. The caller
//! authenticates the gate's consensus context and predecessor.
//!
//! The [`l1`] module binds exact raw L1 headers and their ancestry to an
//! independently authenticated reference. It supplies covered header roots to
//! later witness verification without establishing reference provenance itself.
//!
//! The [`projection`] module follows the revised header/envelope/execution
//! binding. It checks equality against caller-authenticated context and returns
//! exact original RLP spans. Nested witness authentication and the remaining
//! execution, consensus and range checks belong to the composing verifier.
//!
//! The [`statement`] module follows the revised landing design. The full
//! proof must authenticate the deployment, origin, L1 state, committee
//! schedule and `CometBFT` header chain. It must check the current and next
//! validator hashes at every height and verify execution, forced inclusion,
//! data availability and complete range continuity. `ValState` belongs to node
//! validation.

#![no_std]

extern crate alloc;

pub mod l1;
pub mod projection;
pub mod statement;

use alloy_primitives::B256;
use etna_core::{
    comet::{
        codec::{CodecError, decode_canonical_block, decode_canonical_commit},
        native::{Block, Commit},
        validators::Validators,
        verify::{BftError, CertifiedBlockRef, verify_body_at_origin, verify_native_commit},
    },
    domain::{DeploymentDomain, DomainError, chain_id},
};
use tendermint::block::{Header, Id};

/// Byte budgets set by the outer verifier before decoding untrusted protobuf.
#[derive(Clone, Copy)]
pub struct CometWireLimits {
    /// Maximum original block bytes, including its embedded `LastCommit`.
    pub block_bytes: usize,
    /// Maximum bytes of the separately supplied current-height certificate.
    pub commit_bytes: usize,
}

/// The untrusted `CometBFT` input exceeds its budget or fails canonical decoding.
#[derive(Debug, thiserror::Error)]
pub enum CometBlockDecodeError {
    /// The raw block exceeds the caller's pre-decode budget.
    #[error("Comet block has {actual} bytes, exceeding limit {max}")]
    BlockTooLarge {
        /// Received bytes.
        actual: usize,
        /// Maximum allowed bytes.
        max: usize,
    },
    /// The standalone certificate exceeds the caller's pre-decode budget.
    #[error("Comet commit has {actual} bytes, exceeding limit {max}")]
    CommitTooLarge {
        /// Received bytes.
        actual: usize,
        /// Maximum allowed bytes.
        max: usize,
    },
    /// The original block or its structural projection failed canonical decoding.
    #[error("invalid Comet block: {0}")]
    Block(#[source] CodecError),
    /// The standalone certificate failed canonical decoding.
    #[error("invalid Comet commit: {0}")]
    Commit(#[source] CodecError),
}

/// Canonically decoded `CometBFT` witness awaiting consensus verification.
pub struct CometBlock<'a> {
    raw_block: &'a [u8],
    block: Block,
    commit: Commit,
}

impl<'a> CometBlock<'a> {
    /// Bounds and decodes the exact block and current certificate protobuf bytes.
    ///
    /// The caller enforces outer witness framing and byte budgets before any
    /// transport or proof-input allocation. This constructor retains a private
    /// structural projection of the original block.
    ///
    /// # Errors
    ///
    /// Returns an error if either byte budget is exceeded or either input fails
    /// the canonical `CometBFT` codec.
    pub fn decode(
        raw_block: &'a [u8],
        raw_commit: &[u8],
        limits: CometWireLimits,
    ) -> Result<Self, CometBlockDecodeError> {
        if raw_block.len() > limits.block_bytes {
            return Err(CometBlockDecodeError::BlockTooLarge {
                actual: raw_block.len(),
                max: limits.block_bytes,
            });
        }
        if raw_commit.len() > limits.commit_bytes {
            return Err(CometBlockDecodeError::CommitTooLarge {
                actual: raw_commit.len(),
                max: limits.commit_bytes,
            });
        }

        let (_, block) = decode_canonical_block(raw_block).map_err(CometBlockDecodeError::Block)?;
        let commit = decode_canonical_commit(raw_commit).map_err(CometBlockDecodeError::Commit)?;
        Ok(Self {
            raw_block,
            block,
            commit,
        })
    }
}

/// Inputs the caller must independently authenticate before invoking the consensus gate.
///
/// Derive the deployment, origin and expected parent from the established chain
/// and proof range, and bind the validators to this height's authorized epoch
/// using the L1 registry and validator state. Header self-consistency and a
/// host-supplied committee record provide no such authorization.
#[derive(Clone, Copy)]
pub struct ConsensusContext<'a> {
    /// Deployment and generation from authenticated protocol context.
    pub domain: DeploymentDomain,
    /// Expected current height, derived from the authenticated parent/range.
    pub height: u64,
    /// Authenticated parent height of the generation's first Comet block.
    pub origin_height: u64,
    /// Authenticated execution parent hash to compare with the Comet app hash.
    pub parent_el_hash: B256,
    /// Active validator set independently authorized at the expected height.
    pub validators: &'a Validators,
}

/// The caller's authenticated Comet predecessor context at the preceding height.
///
/// The outer verifier binds this predecessor to the very same parent as the EL
/// hash and authenticates the active set for that predecessor's epoch. Within a
/// range, use the signed header and full `BlockID` from the previous gate result.
/// At a checkpoint, authenticate the identity through the start context.
#[derive(Clone, Copy)]
pub struct PreviousComet<'a> {
    /// Authenticated previous header used to verify the embedded `LastCommit`.
    pub header: &'a Header,
    /// Authenticated previous header hash and original Comet part-set identity.
    pub block_id: &'a Id,
    /// Active validator set independently authorized at the previous height.
    pub validators: &'a Validators,
}

/// The borrowed transaction and signed Comet identity established by this gate.
#[derive(Debug)]
pub struct CertifiedEnvelope<'a> {
    envelope: &'a [u8],
    header: &'a Header,
    block_id: &'a Id,
}

impl<'a> CertifiedEnvelope<'a> {
    /// The exact committed Comet transaction bytes for Etna envelope decoding.
    #[must_use]
    pub const fn envelope_bytes(&self) -> &'a [u8] {
        self.envelope
    }

    /// The header signed by the current native certificate.
    #[must_use]
    pub const fn header(&self) -> &'a Header {
        self.header
    }

    /// The signed header hash and original Comet part-set identity.
    #[must_use]
    pub const fn block_id(&self) -> &'a Id {
        self.block_id
    }
}

/// Supplied input failed the Etna consensus gate.
#[derive(Debug, thiserror::Error)]
pub enum CertifiedEnvelopeError {
    /// The deployment and generation cannot form a canonical Etna chain ID.
    #[error("invalid Etna deployment domain: {0}")]
    Domain(#[source] DomainError),
    /// Current height, parent EL hash or a body commitment disagreed with the witness.
    #[error("invalid Comet body: {0}")]
    Body(#[source] BftError),
    /// The current certificate disagreed with the header, domain or current committee.
    #[error("invalid current Comet certificate: {0}")]
    CurrentCertificate(#[source] BftError),
    /// The generation's first height forbids a predecessor, and later heights require one.
    #[error("unexpected or missing Comet predecessor")]
    PreviousPresence,
    /// The embedded `LastCommit`'s full `BlockID` differed from the authenticated predecessor.
    #[error("embedded LastCommit has a different Comet predecessor BlockID")]
    PreviousIdentity,
    /// The actual embedded `LastCommit` failed native QC verification at the preceding height.
    #[error("invalid embedded Comet LastCommit: {0}")]
    EmbeddedCertificate(#[source] BftError),
}

/// Verifies a certified Comet body against independently authenticated caller context.
///
/// The current body binds exact data, evidence, part set, height, parent EL hash
/// and generation-start ancestry. The current QC authenticates that identity.
/// After origin + 1, the actual embedded `LastCommit` must match the authenticated
/// predecessor's full `BlockID` and pass native QC against its header and active
/// validator set. Different valid signature sets for the same `BlockID` are allowed.
///
/// # Errors
///
/// Returns an error on an invalid domain, current body or QC, or missing,
/// unexpected, mismatched or invalid predecessor ancestry.
pub fn verify_certified_envelope<'a>(
    block: &'a CometBlock<'_>,
    context: &ConsensusContext<'_>,
    previous: Option<PreviousComet<'_>>,
) -> Result<CertifiedEnvelope<'a>, CertifiedEnvelopeError> {
    let chain_id = chain_id(&context.domain).map_err(CertifiedEnvelopeError::Domain)?;
    let current = CertifiedBlockRef {
        raw_block: block.raw_block,
        block: &block.block,
        commit: &block.commit,
    };
    let envelope = verify_body_at_origin(
        current,
        context.height,
        context.parent_el_hash,
        context.origin_height,
    )
    .map_err(CertifiedEnvelopeError::Body)?;

    match (&current.block.last_commit, previous) {
        (None, None) => {}
        (Some(embedded), Some(previous)) => {
            if &embedded.block_id != previous.block_id {
                return Err(CertifiedEnvelopeError::PreviousIdentity);
            }
            verify_native_commit(embedded, previous.header, previous.validators, &chain_id)
                .map_err(CertifiedEnvelopeError::EmbeddedCertificate)?;
        }
        _ => return Err(CertifiedEnvelopeError::PreviousPresence),
    }
    verify_native_commit(
        current.commit,
        &current.block.header,
        context.validators,
        &chain_id,
    )
    .map_err(CertifiedEnvelopeError::CurrentCertificate)?;

    Ok(CertifiedEnvelope {
        envelope,
        header: &current.block.header,
        block_id: &current.commit.block_id,
    })
}
