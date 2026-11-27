use super::{BeaconHeader, L1HistoryError, SszProofNode};
use alloc::collections::BTreeMap;
use alloy_primitives::B256;
use sha2::{Digest, Sha256};

pub(super) type Leaves = BTreeMap<u64, B256>;

pub(super) fn pair(left: B256, right: B256) -> B256 {
    B256::from(<[u8; 32]>::from(
        Sha256::new()
            .chain_update(left)
            .chain_update(right)
            .finalize(),
    ))
}

pub(super) fn scalar(value: u64) -> B256 {
    let mut root = B256::ZERO;
    root[..8].copy_from_slice(&value.to_le_bytes());
    root
}

pub(super) fn header_root(header: &BeaconHeader) -> B256 {
    let left = pair(
        pair(scalar(header.slot), scalar(header.proposer_index)),
        pair(header.parent_root, header.state_root),
    );
    let right = pair(
        pair(header.body_root, B256::ZERO),
        pair(B256::ZERO, B256::ZERO),
    );
    pair(left, right)
}

pub(super) fn open(leaves: &mut Leaves, index: u64, root: B256) -> Result<(), L1HistoryError> {
    if index == 0 || leaves.insert(index, root).is_some() {
        return Err(L1HistoryError::Merkle);
    }
    Ok(())
}

// Starting with the largest index always processes descendants before their parents.
// Consume a helper only when the required sibling cannot be derived from another opening.
// The expected helper indices encountered this way are strictly decreasing.
pub(super) fn verify(
    mut leaves: Leaves,
    helpers: &[SszProofNode],
    expected: B256,
) -> Result<(), L1HistoryError> {
    let mut helpers = helpers.iter().rev();
    while let Some((index, node)) = leaves.pop_last() {
        if index == 1 {
            return if helpers.next().is_none() && node == expected {
                Ok(())
            } else {
                Err(L1HistoryError::Merkle)
            };
        }
        let sibling_index = index ^ 1;
        let sibling = if let Some(root) = leaves.remove(&sibling_index) {
            root
        } else {
            helpers
                .next()
                .filter(|helper| helper.index == sibling_index)
                .ok_or(L1HistoryError::Merkle)?
                .root
        };
        let parent = if index & 1 == 0 {
            pair(node, sibling)
        } else {
            pair(sibling, node)
        };
        open(&mut leaves, index / 2, parent)?;
    }
    Err(L1HistoryError::Merkle)
}

pub(super) fn verify_branch(
    mut index: u64,
    mut node: B256,
    siblings: &[B256],
    expected: B256,
) -> Result<(), L1HistoryError> {
    if siblings.len() != index.ilog2() as usize {
        return Err(L1HistoryError::Merkle);
    }
    for &sibling in siblings {
        node = if index & 1 == 0 {
            pair(node, sibling)
        } else {
            pair(sibling, node)
        };
        index /= 2;
    }
    if node != expected {
        return Err(L1HistoryError::Merkle);
    }
    Ok(())
}
