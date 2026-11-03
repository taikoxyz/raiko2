//! Experimental `ZisK` wire adapter; no production backend is registered.

use alloy_primitives::{Address, B256};
use anyhow::{Result, ensure};
use raiko2_protocol_shasta::shasta::ProofCarryData;
use serde::{Deserialize, Serialize};

/// Child proof words and protocol carry data. Keys never come from stdin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZiskAggregationInput {
    pub proofs: Vec<Vec<u64>>,
    pub proof_carry_data_vec: Vec<ProofCarryData>,
    pub prover_address: Address,
}

/// Wrap a bincode 1.3 payload in one `ZisK` raw stdin frame.
#[must_use]
pub fn frame_stdin(payload: &[u8]) -> Vec<u8> {
    let mut frame = (payload.len() as u64).to_le_bytes().to_vec();
    frame.extend_from_slice(payload);
    frame.resize(frame.len().next_multiple_of(8), 0);
    frame
}

/// Validate exactly one raw frame, including its zero alignment padding.
///
/// # Errors
/// Returns an error for truncated, oversized, extra, or nonzero-padded input.
pub fn unframe_stdin(frame: &[u8]) -> Result<&[u8]> {
    ensure!(frame.len() >= 8, "truncated stdin length");
    let length = usize::try_from(u64::from_le_bytes(frame[..8].try_into()?))?;
    let end = length
        .checked_add(8)
        .ok_or_else(|| anyhow::anyhow!("stdin length overflow"))?;
    let padded = end
        .checked_add(7)
        .ok_or_else(|| anyhow::anyhow!("stdin padding overflow"))?
        & !7;
    ensure!(frame.len() == padded, "stdin frame length mismatch");
    ensure!(
        frame[end..].iter().all(|byte| *byte == 0),
        "nonzero stdin padding"
    );
    Ok(&frame[8..end])
}

/// Recover the 32 committed bytes from 64 public u32 slots represented as u64.
///
/// # Errors
/// Returns an error for wrong slot counts, values exceeding u32, or nonzero padding.
pub fn digest_from_public_slots(slots: &[u64]) -> Result<[u8; 32]> {
    ensure!(slots.len() == 64, "expected 64 public slots");
    ensure!(
        slots[8..].iter().all(|slot| *slot == 0),
        "nonzero unused public slots"
    );
    let mut digest = [0; 32];
    for (chunk, slot) in digest.chunks_exact_mut(4).zip(slots) {
        chunk.copy_from_slice(&u32::try_from(*slot)?.to_le_bytes());
    }
    Ok(digest)
}

/// Decode a full leaf proof's public prefix. This does NOT verify the proof.
/// Call only together with the pinned `ZisK` cryptographic verifier; its success
/// authenticates these same words. Compressed/fold proofs are deliberately unsupported.
///
/// # Errors
/// Returns an error for malformed public prefixes or a different child program.
pub fn digest_from_proof(proof: &[u64], expected_program_vk: &[u64; 4]) -> Result<[u8; 32]> {
    // [minimal=0][n_publics=69][leaf=1][program_vk(4)][publics(64)]
    // followed by nonempty proof data and the appended [setup_vk(4)][hash_tag].
    ensure!(proof.len() >= 77, "truncated full leaf proof");
    ensure!(
        proof[..3] == [0, 69, 1],
        "expected non-compressed leaf proof"
    );
    ensure!(
        proof[3..7] == *expected_program_vk,
        "child program VK mismatch"
    );
    digest_from_public_slots(&proof[7..71])
}

/// Experimental program ID: four canonical VK limbs in little-endian order.
/// No on-chain `ZisK` verifier contract currently consumes this convention.
#[must_use]
pub fn program_id(program_vk: &[u64; 4]) -> B256 {
    let mut bytes = [0; 32];
    for (chunk, limb) in bytes.chunks_exact_mut(8).zip(program_vk) {
        chunk.copy_from_slice(&limb.to_le_bytes());
    }
    B256::from(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_frame_preserves_bincode_one_and_eight_byte_alignment() {
        let bytes = bincode::serialize(&vec![1_u8, 2, 3]).unwrap();
        let frame = frame_stdin(&bytes);
        assert_eq!(&frame[..8], &11_u64.to_le_bytes());
        assert_eq!(frame.len(), 24);
        assert_eq!(unframe_stdin(&frame).unwrap(), bytes);
        assert_eq!(
            bincode::deserialize::<Vec<u8>>(unframe_stdin(&frame).unwrap()).unwrap(),
            [1, 2, 3]
        );
    }

    #[test]
    fn raw_frame_rejects_truncated_extra_and_nonzero_padding() {
        let frame = frame_stdin(&[1, 2, 3]);
        assert!(unframe_stdin(&frame[..15]).is_err());
        assert!(unframe_stdin(&[0; 7]).is_err());
        let mut bad = frame.clone();
        bad[15] = 1;
        assert!(unframe_stdin(&bad).is_err());
        let mut extra = frame;
        extra.extend_from_slice(&[0; 8]);
        assert!(unframe_stdin(&extra).is_err());
        assert!(unframe_stdin(&u64::MAX.to_le_bytes()).is_err());
    }

    #[test]
    fn public_digest_is_little_endian_and_requires_zero_unused_slots() {
        let mut slots = [0_u64; 64];
        slots[0] = 0x0403_0201;
        let digest = digest_from_public_slots(&slots).unwrap();
        assert_eq!(&digest[..4], &[1, 2, 3, 4]);
        assert!(digest_from_public_slots(&slots[..63]).is_err());
        slots[8] = 1;
        assert!(digest_from_public_slots(&slots).is_err());
        slots[8] = 0;
        slots[0] = 1_u64 << 32;
        assert!(digest_from_public_slots(&slots).is_err());
    }

    #[test]
    fn proof_publics_are_bound_to_expected_leaf_program() {
        let key = [11, 22, 33, 44];
        let mut proof = vec![0; 77];
        proof[..3].copy_from_slice(&[0, 69, 1]);
        proof[3..7].copy_from_slice(&key);
        proof[7] = 0x0403_0201;
        assert_eq!(
            &digest_from_proof(&proof, &key).unwrap()[..4],
            &[1, 2, 3, 4]
        );
        assert!(digest_from_proof(&proof, &[0; 4]).is_err());
        assert!(digest_from_proof(&proof[..71], &key).is_err());
        proof[2] = 0; // A folded subtree is not a proposal child.
        assert!(digest_from_proof(&proof, &key).is_err());
        proof[2] = 1;
        proof[0] = 2;
        assert!(digest_from_proof(&proof, &key).is_err());
    }
}
