//! Shared committee arithmetic and origin storage layout for native consensus composition.
//!
//! This module never treats supplied storage words as authenticated. The guest composer reads
//! them with the existing sparse-state reader under its verified L1/L2 headers, then uses these
//! pure helpers. A signer source label never contributes to the validator commitment.

use alloy_primitives::{B256, U256, keccak256};
use etna_core::{
    comet::validators::Validators, committee::decide, fallback::FallbackKeys, params::UINT48_MAX,
    staking::StakingWords, types::OriginRecord,
};

/// Incorrect origin words, fallback preimage, or signer metadata.
#[derive(Debug, thiserror::Error)]
pub enum ConsensusError {
    /// An origin word exceeds its protocol width or disagrees with the authenticated generation.
    #[error("invalid generation origin record")]
    Origin,
    /// The supplied fallback key list does not open the applicable storage commitment.
    #[error("fallback keys do not match authenticated commitment")]
    FallbackHash,
    /// The derived native set cannot satisfy the canonical native validator constraints.
    #[error("invalid derived native validators")]
    Validators,
}

/// The four consecutive Inbox origin words at mapping slot 290 for this generation.
#[must_use]
pub fn origin_slots(generation: u64) -> [B256; 4] {
    let mut preimage = [0u8; 64];
    preimage[..32].copy_from_slice(&U256::from(generation).to_be_bytes::<32>());
    preimage[32..].copy_from_slice(&U256::from(290).to_be_bytes::<32>());
    let base = U256::from_be_bytes(keccak256(preimage).0);
    core::array::from_fn(|offset| B256::from(base.wrapping_add(U256::from(offset))))
}

/// Checks a proven origin record and its fallback preimage against the range's generation origin.
///
/// # Errors
/// Rejects non-uint48 origin fields, an unexpected origin height, and a fallback hash mismatch.
pub fn authenticate_origin(
    words: &[U256; 4],
    expected_height: u64,
    fallback: &FallbackKeys,
) -> Result<OriginRecord, ConsensusError> {
    let narrow = |word| {
        u64::try_from(word)
            .ok()
            .filter(|height| *height <= UINT48_MAX)
    };
    let start_height = narrow(words[0]).ok_or(ConsensusError::Origin)?;
    let l1_number = narrow(words[1]).ok_or(ConsensusError::Origin)?;
    let l1_timestamp = narrow(words[2]).ok_or(ConsensusError::Origin)?;
    if start_height != expected_height {
        return Err(ConsensusError::Origin);
    }
    let fallback_keys_hash = B256::from(words[3]);
    if fallback.hash() != fallback_keys_hash {
        return Err(ConsensusError::FallbackHash);
    }
    Ok(OriginRecord {
        start_height,
        l1_number,
        l1_timestamp,
        fallback_keys_hash,
    })
}

/// Applies the authoritative shared-core decision to the complete authenticated snapshot.
///
/// # Errors
/// Rejects a fallback list that does not open the deciding anchor's slot 278, and native sets
/// that cannot satisfy the canonical key/address/power constraints. An unusable candidate is
/// ordinary fallback, including a stale full-width height or malformed high record bits.
pub fn derive_signers(
    deciding_height: u64,
    snapshot: &StakingWords,
    min_total_stake: U256,
    fallback_hash: B256,
    fallback: &FallbackKeys,
) -> Result<Validators, ConsensusError> {
    if fallback.hash() != fallback_hash {
        return Err(ConsensusError::FallbackHash);
    }
    Validators::from_members(&decide(deciding_height, snapshot, min_total_stake, fallback).members)
        .map_err(|_| ConsensusError::Validators)
}

#[cfg(test)]
mod tests;
