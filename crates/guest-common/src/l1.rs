//! Shared linkage of ascending execution headers to a trusted terminal hash.

use alloy_primitives::B256;
use anyhow::{ensure, Context, Result};

#[expect(
    clippy::redundant_pub_crate,
    reason = "Keep shared helpers crate-private under the unreachable_pub lint"
)]
pub(crate) struct L1HeaderLink {
    pub(crate) number: u64,
    pub(crate) hash: B256,
    pub(crate) parent_hash: B256,
}

// Fork-specific decoding, checkpoint checks and resource policies stay with the
// caller. Every supplied header participates in the walk, including any prefix
// or suffix beyond the caller's consumed checkpoints.
#[expect(
    clippy::redundant_pub_crate,
    reason = "Keep shared helpers crate-private under the unreachable_pub lint"
)]
pub(crate) fn verify_l1_header_chain(
    headers: impl IntoIterator<Item = Result<L1HeaderLink>>,
    expected_number: u64,
    expected_hash: B256,
    label: &str,
) -> Result<()> {
    let mut previous: Option<L1HeaderLink> = None;
    for (index, header) in headers.into_iter().enumerate() {
        let header = header?;
        if let Some(parent) = &previous {
            ensure!(
                parent.number.checked_add(1) == Some(header.number),
                "{label} must be contiguous at index {index}"
            );
            ensure!(
                header.parent_hash == parent.hash,
                "{label} parent hash mismatch at index {index}"
            );
        }
        previous = Some(header);
    }
    let last = previous.with_context(|| format!("{label} must not be empty"))?;
    ensure!(
        last.number == expected_number,
        "{label} last block number mismatch: expected {expected_number}, got {}",
        last.number
    );
    ensure!(last.hash == expected_hash, "{label} last hash mismatch");
    Ok(())
}
