//! Explicit host-only transaction envelopes and gates on actual committed state.
use super::access_list::ProgramAccessList;
use alloy_consensus::{Transaction as _, TrieAccount};
use alloy_primitives::{Address, B256, U256, keccak256};
use anyhow::{Result, ensure};
use reth_ethereum_primitives::TransactionSigned;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, ops::Range};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TransactionKind {
    Call,
    Create,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramTransactions {
    pub kind: TransactionKind,
    #[serde(
        default,
        deserialize_with = "present_count",
        skip_serializing_if = "Option::is_none"
    )]
    pub count_per_block: Option<usize>,
    #[serde(
        default,
        deserialize_with = "present_counts",
        skip_serializing_if = "Option::is_none"
    )]
    pub counts_by_block: Option<Vec<usize>>,
    pub gas_limit: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_runtime_hex: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recipients: Option<Vec<Address>>,
    /// Exact nonce-indexed lists for one-block CALL only; exclusive with the shared list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_lists: Option<Vec<ProgramAccessList>>,
}

#[derive(Debug, Serialize)]
pub struct TransactionsReport {
    pub kind: TransactionKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count_per_block: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub counts_by_block: Option<Vec<usize>>,
    pub gas_limit: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_runtime_hex: Option<String>,
    pub blocks: Vec<TransactionBlockReport>,
}
#[derive(Debug, Serialize)]
pub struct TransactionBlockReport {
    pub block_index: usize,
    /// Accounts are block-final poststates, not intermediate states after each receipt.
    pub users: Vec<TransactionUserReport>,
}
#[derive(Debug, Serialize)]
pub struct TransactionUserReport {
    pub transaction_index: usize,
    pub nonce: u64,
    pub gas_limit: u64,
    pub to: Option<Address>,
    pub transaction_hash: B256,
    pub create_address: Option<Address>,
    pub poststate: Option<TransactionAccountPostState>,
}
#[derive(Debug, Serialize)]
pub struct TransactionAccountPostState {
    pub nonce: u64,
    pub balance: U256,
    pub code_hash: B256,
}

// Present null is invalid, preserving the legacy required-count scalar contract.
fn present_count<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<usize>, D::Error> {
    usize::deserialize(d).map(Some)
}
fn present_counts<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Vec<usize>>, D::Error> {
    Vec::<usize>::deserialize(d).map(Some)
}

/// The implicit legacy envelope remains exactly one user per block.
pub(super) fn user_schedule(
    transactions: Option<&ProgramTransactions>,
    block_count: usize,
) -> Result<Vec<Range<usize>>> {
    ensure!(
        (1..=32).contains(&block_count),
        "transactions block count exceeds bounds"
    );
    match transactions {
        Some(t) => t.schedule(block_count),
        None => Ok((0..block_count).map(|i| i..i + 1).collect()),
    }
}

impl ProgramTransactions {
    /// One checked prefix schedule for signing, block slices, global nonces and reporting.
    pub(super) fn schedule(&self, block_count: usize) -> Result<Vec<Range<usize>>> {
        ensure!(
            (1..=32).contains(&block_count),
            "transactions block count exceeds bounds"
        );
        let counts = match (&self.count_per_block, &self.counts_by_block) {
            (Some(n), None) => {
                ensure!(
                    (1..=2048).contains(n),
                    "transactions count_per_block must be in 1..=2048"
                );
                vec![*n; block_count]
            }
            (None, Some(counts)) => {
                ensure!(
                    counts.len() == block_count,
                    "counts_by_block length must equal block_count"
                );
                ensure!(
                    counts.iter().all(|n| *n <= 2048),
                    "counts_by_block entries must be in 0..=2048"
                );
                counts.clone()
            }
            _ => anyhow::bail!(
                "transactions requires exactly one of count_per_block or counts_by_block"
            ),
        };
        let mut end = 0usize;
        let ranges = counts
            .into_iter()
            .map(|n| {
                let start = end;
                end = start
                    .checked_add(n)
                    .ok_or_else(|| anyhow::anyhow!("transaction count overflow"))?;
                Ok(start..end)
            })
            .collect::<Result<Vec<_>>>()?;
        ensure!(
            end > 0,
            "transactions proposal must contain at least one user"
        );
        Ok(ranges)
    }

    /// One source of truth for signing and committed-envelope/poststate checks.
    pub(super) fn call_recipient(&self, index: usize) -> Result<Address> {
        ensure!(
            self.kind == TransactionKind::Call,
            "CALL recipient requested for CREATE"
        );
        match &self.recipients {
            Some(recipients) => recipients
                .get(index)
                .copied()
                .ok_or_else(|| anyhow::anyhow!("transactions recipient index out of bounds")),
            None => Ok(Address::repeat_byte(0x66)),
        }
    }

    /// The signing and committed-list gate use the same checked user position.
    pub(super) fn access_list_for_user<'a>(
        &'a self,
        index: usize,
        shared: Option<&'a ProgramAccessList>,
    ) -> Result<Option<&'a ProgramAccessList>> {
        match &self.access_lists {
            Some(lists) => {
                ensure!(
                    shared.is_none(),
                    "per-transaction and shared access lists are exclusive"
                );
                Ok(Some(lists.get(index).ok_or_else(|| {
                    anyhow::anyhow!("access-list user index out of bounds")
                })?))
            }
            None => Ok(shared),
        }
    }

    pub(super) fn validate(
        &self,
        code: &[u8],
        input: &[u8],
        block_count: usize,
        storage: bool,
        authorizations: bool,
        access_list: bool,
    ) -> Result<()> {
        self.schedule(block_count)?;
        ensure!(
            self.counts_by_block.is_none()
                || (self.kind == TransactionKind::Call
                    && !storage
                    && !authorizations
                    && !access_list
                    && self.access_lists.is_none()),
            "counts_by_block requires plain CALL without storage_state, access lists or authorizations"
        );
        ensure!(
            (21_000..=15_000_000).contains(&self.gas_limit),
            "transactions gas_limit must be in 21000..=15000000"
        );
        ensure!(
            !authorizations,
            "transactions cannot be combined with authorizations"
        );
        ensure!(
            !access_list || (self.kind == TransactionKind::Call && !storage),
            "transactions access_list requires CALL without storage_state"
        );
        ensure!(
            !storage
                || (self.kind == TransactionKind::Call
                    && block_count == 1
                    && self.count_per_block.is_some_and(|n| n <= 8)),
            "storage_state transactions require CALL, one block and 1..=8 users"
        );
        if let Some(lists) = &self.access_lists {
            ensure!(
                self.kind == TransactionKind::Call
                    && block_count == 1
                    && !storage
                    && !authorizations,
                "per-transaction access_lists requires one-block CALL without storage_state or authorizations"
            );
            ensure!(
                !access_list,
                "per-transaction and shared access lists are exclusive"
            );
            ensure!(
                Some(lists.len()) == self.count_per_block,
                "access_lists length must equal count_per_block"
            );
            let mut items = 0_usize;
            let mut keys = 0_usize;
            for descriptor in lists {
                let list = descriptor.canonical_list()?;
                items = items
                    .checked_add(list.0.len())
                    .ok_or_else(|| anyhow::anyhow!("access-list total overflow"))?;
                for item in &list.0 {
                    keys = keys
                        .checked_add(item.storage_keys.len())
                        .ok_or_else(|| anyhow::anyhow!("access-list total overflow"))?;
                }
                ensure!(
                    items <= 16_384 && keys <= 16_384,
                    "per-transaction access_lists totals exceed16384 items or keys"
                );
            }
        }
        match self.kind {
            TransactionKind::Call => {
                ensure!(
                    !code.is_empty() && code.len() <= 24_576 && input.len() <= 1_048_576,
                    "call requires 1..=24576 code bytes and at most 1MiB calldata"
                );
                ensure!(
                    self.expected_runtime_hex.is_none(),
                    "call cannot specify expected_runtime_hex"
                );
            }
            TransactionKind::Create => {
                ensure!(
                    code.len() <= 49_152 && input.is_empty(),
                    "create requires at most 49152 initcode bytes and empty input_hex"
                );
                if let Some(runtime) = &self.expected_runtime_hex {
                    ensure!(
                        super::program_bytes(runtime)?.len() <= 24_576,
                        "expected runtime exceeds 24576 bytes"
                    );
                }
            }
        }
        Ok(())
    }
    pub(super) fn before_block(
        &self,
        user_range: Range<usize>,
        sender: Address,
        accounts: &BTreeMap<B256, TrieAccount>,
    ) -> Result<()> {
        let nonce = u64::try_from(user_range.start)?;
        ensure!(
            accounts.get(&keccak256(sender)).map(|a| a.nonce) == Some(nonce),
            "unexpected transaction sender prestate nonce"
        );
        if self.kind == TransactionKind::Create {
            for position in user_range {
                ensure!(
                    !accounts.contains_key(&keccak256(sender.create(u64::try_from(position)?))),
                    "create address must be absent before its block"
                );
            }
        }
        Ok(())
    }
    pub(super) fn after_block(
        &self,
        block_index: usize,
        user_range: Range<usize>,
        sender: Address,
        transactions: &[TransactionSigned],
        accounts: &BTreeMap<B256, TrieAccount>,
    ) -> Result<TransactionBlockReport> {
        ensure!(
            transactions.len() == user_range.len(),
            "unexpected committed user count"
        );
        let expected_hash = self
            .expected_runtime_hex
            .as_deref()
            .map(super::program_bytes)
            .transpose()?
            .map(keccak256);
        let mut users = Vec::with_capacity(user_range.len());
        for (index, tx) in transactions.iter().enumerate() {
            let position = user_range
                .start
                .checked_add(index)
                .ok_or_else(|| anyhow::anyhow!("user position overflow"))?;
            let nonce = u64::try_from(position)?;
            let expected_to = (self.kind == TransactionKind::Call)
                .then(|| self.call_recipient(position))
                .transpose()?;
            ensure!(
                tx.nonce() == nonce && tx.to() == expected_to && tx.gas_limit() == self.gas_limit,
                "committed transaction envelope differs from descriptor"
            );
            let create_address =
                (self.kind == TransactionKind::Create).then(|| sender.create(nonce));
            let address = create_address
                .or(expected_to)
                .expect("call or create destination");
            let account = accounts.get(&keccak256(address));
            if self.kind == TransactionKind::Create {
                match expected_hash {
                    Some(hash) => ensure!(
                        account.is_some_and(|a| a.nonce == 1 && a.code_hash == hash),
                        "created account runtime or nonce differs from expectation"
                    ),
                    None => ensure!(
                        account.is_none(),
                        "created account must be absent without expected_runtime_hex"
                    ),
                }
            }
            users.push(TransactionUserReport {
                transaction_index: index + 1,
                nonce,
                gas_limit: tx.gas_limit(),
                to: tx.to(),
                transaction_hash: *tx.hash(),
                create_address,
                poststate: account.map(|a| TransactionAccountPostState {
                    nonce: a.nonce,
                    balance: a.balance,
                    code_hash: a.code_hash,
                }),
            });
        }
        ensure!(
            accounts.get(&keccak256(sender)).map(|a| a.nonce)
                == Some(u64::try_from(user_range.end)?),
            "unexpected transaction sender poststate nonce"
        );
        Ok(TransactionBlockReport { block_index, users })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonuniform_call_schedule_preserves_empty_blocks_and_global_positions() {
        let descriptor: ProgramTransactions = serde_json::from_value(serde_json::json!({
            "kind":"call", "counts_by_block":[0,2,0,1], "gas_limit":100000
        }))
        .expect("nonuniform CALL descriptor must parse");
        descriptor
            .validate(&[0], &[], 4, false, false, false)
            .unwrap();
        assert_eq!(
            descriptor.schedule(4).unwrap(),
            vec![0..0, 0..2, 2..2, 2..3]
        );
        let uniform: ProgramTransactions = serde_json::from_value(serde_json::json!({
            "kind":"call", "count_per_block":2, "gas_limit":100000
        }))
        .unwrap();
        assert_eq!(uniform.schedule(3).unwrap(), vec![0..2, 2..4, 4..6]);
        assert!(uniform.schedule(0).is_err());
        assert!(uniform.schedule(33).is_err());
        let maximum: ProgramTransactions = serde_json::from_value(serde_json::json!({
            "kind":"call", "counts_by_block":vec![2048;32], "gas_limit":100000
        }))
        .unwrap();
        assert_eq!(maximum.schedule(32).unwrap().last().unwrap().end, 65536);
        let duplicate =
            r#"{"kind":"call","counts_by_block":[1],"counts_by_block":[1],"gas_limit":100000}"#;
        assert!(serde_json::from_str::<ProgramTransactions>(duplicate).is_err());
    }

    #[test]
    fn schedule_requires_exactly_one_strict_bounded_positive_total_mode() {
        let base = serde_json::json!({"kind":"call", "gas_limit":100000});
        for (uniform, counts, blocks) in [
            (None, None, 2),
            (Some(1), Some(serde_json::json!([0, 1])), 2),
            (Some(0), None, 1),
            (Some(2049), None, 1),
            (None, Some(serde_json::json!([])), 1),
            (None, Some(serde_json::json!([0, 0])), 2),
            (None, Some(serde_json::json!([1])), 2),
            (None, Some(serde_json::json!([0, 2049])), 2),
        ] {
            let mut value = base.clone();
            if let Some(n) = uniform {
                value["count_per_block"] = serde_json::json!(n);
            }
            if let Some(v) = counts {
                value["counts_by_block"] = v;
            }
            let descriptor: ProgramTransactions = serde_json::from_value(value).unwrap();
            assert!(
                descriptor
                    .validate(&[0], &[], blocks, false, false, false)
                    .is_err()
            );
        }
        for field in ["count_per_block", "counts_by_block"] {
            for bad in [
                serde_json::Value::Null,
                serde_json::json!(true),
                serde_json::json!(1.5),
                serde_json::json!(-1),
            ] {
                let mut value = base.clone();
                value[field] = bad;
                assert!(serde_json::from_value::<ProgramTransactions>(value).is_err());
            }
        }
        let mut value = base;
        value["counts_by_block"] = serde_json::json!([0, 1]);
        let descriptor: ProgramTransactions = serde_json::from_value(value.clone()).unwrap();
        for (storage, auth, al) in [
            (true, false, false),
            (false, true, false),
            (false, false, true),
        ] {
            assert!(
                descriptor
                    .validate(&[0], &[], 2, storage, auth, al)
                    .is_err()
            );
        }
        value["kind"] = serde_json::json!("create");
        let descriptor: ProgramTransactions = serde_json::from_value(value).unwrap();
        assert!(
            descriptor
                .validate(&[0], &[], 2, false, false, false)
                .is_err()
        );
    }

    #[test]
    fn per_transaction_access_list_descriptor_parses_and_validates() {
        let descriptor: ProgramTransactions = serde_json::from_value(serde_json::json!({
            "kind":"call", "count_per_block":2, "gas_limit":100000,
            "access_lists":[{"pattern":"raw_addresses","count":1,"addresses":[Address::ZERO]},
                {"pattern":"raw_addresses","count":2,"addresses":[Address::repeat_byte(1),Address::ZERO]}],
        })).expect("per-transaction access_lists must parse");
        descriptor
            .validate(&[0], &[], 1, false, false, false)
            .unwrap();
    }
    #[test]
    fn per_transaction_access_lists_are_exclusive_bounded_and_indexed() {
        let base = serde_json::json!({"kind":"call","count_per_block":2,"gas_limit":100000,
            "access_lists":[{"pattern":"raw_addresses","count":1,"addresses":[Address::ZERO]},
                {"pattern":"raw_addresses","count":2,"addresses":[Address::repeat_byte(1),Address::ZERO]}]});
        let parse = |v| serde_json::from_value::<ProgramTransactions>(v).unwrap();
        let descriptor = parse(base.clone());
        assert_eq!(
            descriptor
                .access_list_for_user(0, None)
                .unwrap()
                .unwrap()
                .count,
            1
        );
        assert_eq!(
            descriptor
                .access_list_for_user(1, None)
                .unwrap()
                .unwrap()
                .count,
            2
        );
        assert!(descriptor.access_list_for_user(2, None).is_err());
        let shared = descriptor.access_lists.as_ref().unwrap().first().unwrap();
        assert!(descriptor.access_list_for_user(0, Some(shared)).is_err());
        for (field, value) in [
            ("access_lists", serde_json::json!([])),
            (
                "access_lists",
                serde_json::json!([base["access_lists"][0].clone()]),
            ),
            ("count_per_block", serde_json::json!(3)),
            ("kind", serde_json::json!("create")),
        ] {
            let mut value_base = base.clone();
            value_base[field] = value;
            assert!(
                parse(value_base)
                    .validate(&[0], &[], 1, false, false, false)
                    .is_err()
            );
        }
        for (blocks, storage, auth, shared) in [
            (2, false, false, false),
            (1, true, false, false),
            (1, false, true, false),
            (1, false, false, true),
        ] {
            assert!(
                descriptor
                    .validate(&[0], &[], blocks, storage, auth, shared)
                    .is_err()
            );
        }
        for bad in [
            serde_json::json!({}),
            serde_json::json!(true),
            serde_json::json!(1),
        ] {
            let mut value = base.clone();
            value["access_lists"] = bad;
            assert!(serde_json::from_value::<ProgramTransactions>(value).is_err());
        }
        let duplicate = r#"{"kind":"call","count_per_block":1,"gas_limit":100000,"access_lists":[],"access_lists":[]}"#;
        assert!(serde_json::from_str::<ProgramTransactions>(duplicate).is_err());
        for pattern in ["repeated_address", "repeated_key"] {
            let huge = serde_json::json!({"pattern":pattern,"count":8192});
            let mut value = base.clone();
            value["access_lists"] = serde_json::json!([huge.clone(), huge.clone()]);
            assert!(
                parse(value.clone())
                    .validate(&[0], &[], 1, false, false, false)
                    .is_ok()
            );
            value["count_per_block"] = serde_json::json!(3);
            value["access_lists"]
                .as_array_mut()
                .unwrap()
                .push(serde_json::json!({"pattern":pattern,"count":1}));
            assert!(
                parse(value)
                    .validate(&[0], &[], 1, false, false, false)
                    .is_err()
            );
        }
        let legacy = serde_json::json!({"kind":"call","count_per_block":2,"gas_limit":100000});
        let mut null = legacy.clone();
        null["access_lists"] = serde_json::Value::Null;
        assert_eq!(serde_json::to_value(parse(null)).unwrap(), legacy);
    }
}
