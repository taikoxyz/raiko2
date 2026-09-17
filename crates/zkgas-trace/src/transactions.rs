//! Transaction-iterator tracing and outcome classification.

use alloy_consensus::transaction::{Recovered, TxHashRef};
use alloy_primitives::{B256, U256};
use anyhow::{Result, bail};
use reth_ethereum_primitives::TransactionSigned;
use serde::{Deserialize, Serialize};

use crate::TraceSink;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransactionDisposition {
    CommittedSuccess,
    CommittedFailure,
    Attempted,
    Unattempted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveredTransactionOccurrence {
    pub recovered_index: usize,
    pub manifest_index: Option<usize>,
    pub tx_hash: B256,
    pub is_anchor: bool,
}

impl RecoveredTransactionOccurrence {
    #[must_use]
    pub const fn new(
        recovered_index: usize,
        manifest_index: Option<usize>,
        tx_hash: B256,
        is_anchor: bool,
    ) -> Self {
        Self {
            recovered_index,
            manifest_index,
            tx_hash,
            is_anchor,
        }
    }

    #[must_use]
    pub const fn into_unattempted(self) -> TransactionTrace {
        TransactionTrace {
            recovered_index: self.recovered_index,
            started_tx_index: None,
            manifest_index: self.manifest_index,
            tx_hash: self.tx_hash,
            is_anchor: self.is_anchor,
            disposition: TransactionDisposition::Unattempted,
            native_value_transfer: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransactionTrace {
    pub recovered_index: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_tx_index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest_index: Option<usize>,
    pub tx_hash: B256,
    pub is_anchor: bool,
    pub disposition: TransactionDisposition,
    pub native_value_transfer: bool,
}

/// Joins started transaction occurrences to committed hashes as an ordered one-use subsequence.
///
/// # Errors
///
/// Returns an error when a committed transaction cannot be consumed in order.
pub fn classify_started_occurrences(
    started: &[RecoveredTransactionOccurrence],
    committed_hashes: &[B256],
) -> Result<Vec<TransactionTrace>> {
    let mut committed_cursor = 0usize;
    let traces = started
        .iter()
        .enumerate()
        .map(|(started_tx_index, occurrence)| {
            let committed = committed_hashes
                .get(committed_cursor)
                .is_some_and(|hash| *hash == occurrence.tx_hash);
            if committed {
                committed_cursor += 1;
            }
            TransactionTrace {
                recovered_index: occurrence.recovered_index,
                started_tx_index: Some(started_tx_index),
                manifest_index: occurrence.manifest_index,
                tx_hash: occurrence.tx_hash,
                is_anchor: occurrence.is_anchor,
                disposition: if committed {
                    TransactionDisposition::CommittedSuccess
                } else {
                    TransactionDisposition::Attempted
                },
                native_value_transfer: false,
            }
        })
        .collect::<Vec<_>>();

    if committed_cursor != committed_hashes.len() {
        bail!(
            "unconsumed committed transaction at index {committed_cursor}: {} committed rows, {} matched",
            committed_hashes.len(),
            committed_cursor
        );
    }
    Ok(traces)
}

/// Classifies started occurrences and applies the aligned committed receipt statuses.
///
/// # Errors
///
/// Returns an error for a receipt-count mismatch or an unconsumed committed transaction.
pub fn classify_started_occurrences_with_statuses(
    started: &[RecoveredTransactionOccurrence],
    committed_hashes: &[B256],
    committed_statuses: &[bool],
) -> Result<Vec<TransactionTrace>> {
    if committed_hashes.len() != committed_statuses.len() {
        bail!(
            "receipt-count mismatch: {} committed transactions, {} receipts",
            committed_hashes.len(),
            committed_statuses.len()
        );
    }
    let mut committed_cursor = 0usize;
    let traces = started
        .iter()
        .enumerate()
        .map(|(started_tx_index, occurrence)| {
            let committed = committed_hashes
                .get(committed_cursor)
                .is_some_and(|hash| *hash == occurrence.tx_hash);
            let disposition = if committed {
                let status = committed_statuses[committed_cursor];
                committed_cursor += 1;
                if status {
                    TransactionDisposition::CommittedSuccess
                } else {
                    TransactionDisposition::CommittedFailure
                }
            } else {
                TransactionDisposition::Attempted
            };
            TransactionTrace {
                recovered_index: occurrence.recovered_index,
                started_tx_index: Some(started_tx_index),
                manifest_index: occurrence.manifest_index,
                tx_hash: occurrence.tx_hash,
                is_anchor: occurrence.is_anchor,
                disposition,
                native_value_transfer: false,
            }
        })
        .collect::<Vec<_>>();
    if committed_cursor != committed_hashes.len() {
        bail!("unconsumed committed transaction at index {committed_cursor}");
    }
    Ok(traces)
}

#[must_use]
pub fn is_native_value_transfer(
    disposition: TransactionDisposition,
    value: U256,
    is_call: bool,
    has_operation_trace: bool,
) -> bool {
    matches!(disposition, TransactionDisposition::CommittedSuccess)
        && !value.is_zero()
        && is_call
        && !has_operation_trace
}

pub(crate) struct TracingTransactions<'a, I> {
    inner: I,
    sink: TraceSink,
    started: &'a mut Vec<RecoveredTransactionOccurrence>,
    recovered_index: usize,
    manifest_indices: &'a [usize],
}

impl<'a, I> TracingTransactions<'a, I> {
    pub(crate) const fn new(
        inner: I,
        sink: TraceSink,
        started: &'a mut Vec<RecoveredTransactionOccurrence>,
        manifest_indices: &'a [usize],
    ) -> Self {
        Self {
            inner,
            sink,
            started,
            recovered_index: 0,
            manifest_indices,
        }
    }
}

impl<'tx, I> Iterator for TracingTransactions<'_, I>
where
    I: Iterator<Item = Recovered<&'tx TransactionSigned>>,
{
    type Item = Recovered<&'tx TransactionSigned>;

    fn next(&mut self) -> Option<Self::Item> {
        self.sink.lock().finish_transaction();
        let tx = self.inner.next()?;
        let recovered_index = self.recovered_index;
        self.recovered_index += 1;
        let started_tx_index = self.started.len();
        self.started.push(RecoveredTransactionOccurrence {
            recovered_index,
            manifest_index: recovered_index
                .checked_sub(1)
                .and_then(|index| self.manifest_indices.get(index).copied()),
            tx_hash: *tx.inner().tx_hash(),
            is_anchor: recovered_index == 0,
        });
        self.sink.lock().start_transaction(started_tx_index);
        Some(tx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_consensus::{SignableTransaction, TxEip1559};
    use alloy_primitives::{Address, Signature, TxKind};
    use std::{cell::Cell, rc::Rc};

    fn transaction(nonce: u64) -> TransactionSigned {
        TxEip1559 {
            chain_id: 167,
            nonce,
            gas_limit: 21_000,
            max_fee_per_gas: 1,
            max_priority_fee_per_gas: 0,
            to: TxKind::Call(Address::with_last_byte(1)),
            value: U256::ZERO,
            access_list: Vec::new().into(),
            input: alloy_primitives::Bytes::new(),
        }
        .into_signed(Signature::test_signature())
        .into()
    }

    #[test]
    fn iterator_yields_one_transaction_before_requesting_the_next() {
        let transactions = [transaction(0), transaction(1)];
        let pulls = Rc::new(Cell::new(0));
        let pulls_for_iterator = pulls.clone();
        let inner = transactions.iter().map(move |transaction| {
            pulls_for_iterator.set(pulls_for_iterator.get() + 1);
            Recovered::new_unchecked(transaction, Address::ZERO)
        });
        let sink = TraceSink::default();
        let mut started = Vec::new();
        let mut tracing = TracingTransactions::new(inner, sink.clone(), &mut started, &[4, 7]);

        assert_eq!(pulls.get(), 0);
        tracing.next().expect("first transaction");
        assert_eq!(pulls.get(), 1, "the second input must remain lazy");
        sink.lock().record_opcode(0x01, 1, 3, false);

        tracing.next().expect("second transaction");
        assert_eq!(pulls.get(), 2);
        sink.lock().record_opcode(0x02, 1, 3, false);
        assert!(tracing.next().is_none());
        drop(tracing);

        assert_eq!(started.len(), 2);
        assert_eq!(
            started[0].manifest_index, None,
            "anchor has no manifest index"
        );
        assert_eq!(started[1].manifest_index, Some(4));
        let collector = sink.snapshot();
        assert_eq!(collector.operations()[0].tx_index, Some(0));
        assert_eq!(collector.operations()[1].tx_index, Some(1));
    }
}
