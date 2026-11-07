//! Explicit host-only transaction envelopes and gates on actual committed state.
use super::access_list::ProgramAccessList;
use alloy_consensus::{Transaction as _, TrieAccount};
use alloy_primitives::{Address, B256, U256, keccak256};
use anyhow::{Result, ensure};
use reth_ethereum_primitives::TransactionSigned;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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
    pub count_per_block: usize,
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
    pub count_per_block: usize,
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

impl ProgramTransactions {
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
        ensure!(
            (1..=2048).contains(&self.count_per_block),
            "transactions count_per_block must be in 1..=2048"
        );
        ensure!(
            (21_000..=15_000_000).contains(&self.gas_limit),
            "transactions gas_limit must be in 21000..=15000000"
        );
        ensure!(
            (1..=32).contains(&block_count)
                && self.count_per_block.checked_mul(block_count).is_some(),
            "transactions block count exceeds bounds"
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
                    && self.count_per_block <= 8),
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
                lists.len() == self.count_per_block,
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
        block_index: usize,
        sender: Address,
        accounts: &BTreeMap<B256, TrieAccount>,
    ) -> Result<()> {
        let nonce = u64::try_from(block_index * self.count_per_block)?;
        ensure!(
            accounts.get(&keccak256(sender)).map(|a| a.nonce) == Some(nonce),
            "unexpected transaction sender prestate nonce"
        );
        if self.kind == TransactionKind::Create {
            for offset in 0..self.count_per_block {
                ensure!(
                    !accounts
                        .contains_key(&keccak256(sender.create(nonce + u64::try_from(offset)?))),
                    "create address must be absent before its block"
                );
            }
        }
        Ok(())
    }
    pub(super) fn after_block(
        &self,
        block_index: usize,
        sender: Address,
        transactions: &[TransactionSigned],
        accounts: &BTreeMap<B256, TrieAccount>,
    ) -> Result<TransactionBlockReport> {
        ensure!(
            transactions.len() == self.count_per_block,
            "unexpected committed user count"
        );
        let expected_hash = self
            .expected_runtime_hex
            .as_deref()
            .map(super::program_bytes)
            .transpose()?
            .map(keccak256);
        let mut users = Vec::with_capacity(self.count_per_block);
        for (index, tx) in transactions.iter().enumerate() {
            let nonce = u64::try_from(block_index * self.count_per_block + index)?;
            let expected_to = (self.kind == TransactionKind::Call)
                .then(|| self.call_recipient(index))
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
                == Some(u64::try_from((block_index + 1) * self.count_per_block)?),
            "unexpected transaction sender poststate nonce"
        );
        Ok(TransactionBlockReport { block_index, users })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
