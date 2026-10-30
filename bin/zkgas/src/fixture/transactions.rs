//! Explicit host-only transaction envelopes and gates on actual committed state.
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
            !storage && !authorizations && !access_list,
            "transactions cannot be combined with storage_state, authorizations or access_list"
        );
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
            let expected_to =
                (self.kind == TransactionKind::Call).then_some(Address::repeat_byte(0x66));
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
