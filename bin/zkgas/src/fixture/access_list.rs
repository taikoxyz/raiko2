//! Deterministic raw access-list vectors and gates on the actually included signed transaction.
use alloy_consensus::{Transaction as _, transaction::SignerRecoverable};
use alloy_eips::{
    Decodable2718, Encodable2718,
    eip2930::{AccessList, AccessListItem},
};
use alloy_primitives::{Address, B256, U256};
use anyhow::{Result, ensure};
use reth_ethereum_primitives::TransactionSigned;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

const PROGRAM: Address = Address::repeat_byte(0x66);

/// Raw vector shape; multiplicity and order are preserved through signing and execution.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AccessListPattern {
    /// Repeated PROGRAM address items without keys.
    RepeatedAddress,
    /// PROGRAM followed by distinct, initially absent deterministic addresses.
    DistinctAddresses,
    /// One PROGRAM item containing the same storage key repeatedly.
    RepeatedKey,
    /// One PROGRAM item containing ascending keys 1..=count.
    DistinctKeys,
    /// Repeated PROGRAM items, each containing the same single storage key.
    RepeatedItems,
    /// One PROGRAM item containing descending keys count..=1.
    ReversedDistinctKeys,
}

/// Optional bounded access-list description for an otherwise unchanged user transaction.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramAccessList {
    /// Requested raw vector shape.
    pub pattern: AccessListPattern,
    /// Pattern cardinality, in raw address items or storage keys; 1..=8192.
    pub count: usize,
}

/// Requested vector shape and independent observations from included signed transactions.
#[derive(Debug, Serialize)]
pub struct AccessListReport {
    /// Requested raw vector shape.
    pub pattern: AccessListPattern,
    /// Requested cardinality repeated in each block, without deduplication.
    pub count_per_block: usize,
    /// Included user-transaction observations in block order.
    pub blocks: Vec<AccessListBlockReport>,
}

/// Per-block raw and unique selectors read from a canonical decoded transaction.
#[derive(Debug, Serialize)]
pub struct AccessListBlockReport {
    /// Zero-based block index in this constructed proposal.
    pub block_index: usize,
    /// Canonical EIP-2718 envelope type, 2 or 4.
    pub transaction_type: u8,
    /// Raw address item count, including repeated addresses.
    pub raw_address_items: usize,
    /// Raw keys summed over every item, including duplicates.
    pub raw_storage_keys: usize,
    /// Distinct declared addresses; this is not a database-load count.
    pub unique_addresses: usize,
    /// Distinct declared (address, storage key) pairs.
    pub unique_storage_pairs: usize,
}

/// Index zero is PROGRAM; extra addresses occupy a deterministic disjoint fixture prefix.
fn address(index: usize) -> Result<Address> {
    if index == 0 {
        return Ok(PROGRAM);
    }
    let mut bytes = [0_u8; 20];
    bytes[0] = 0xa5;
    bytes[12..].copy_from_slice(&u64::try_from(index)?.to_be_bytes());
    Ok(Address::from(bytes))
}

/// Canonical storage keys are the nonzero 32-byte integers 1..=count.
fn key(index: usize) -> B256 {
    B256::from(U256::from(index + 1).to_be_bytes::<32>())
}

impl ProgramAccessList {
    /// Bounds descriptor allocation; transaction intrinsic gas remains enforced by the EVM.
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (1..=8192).contains(&self.count),
            "access-list count must be in 1..=8192"
        );
        Ok(())
    }

    /// Builds one raw canonical vector without sorting or deduplicating its items or keys.
    pub(super) fn canonical_list(&self) -> Result<AccessList> {
        self.validate()?;
        let item = |address, storage_keys| AccessListItem {
            address,
            storage_keys,
        };
        let list = match self.pattern {
            AccessListPattern::RepeatedAddress => vec![item(PROGRAM, vec![]); self.count],
            AccessListPattern::DistinctAddresses => (0..self.count)
                .map(|index| Ok(item(address(index)?, vec![])))
                .collect::<Result<Vec<_>>>()?,
            AccessListPattern::RepeatedKey => vec![item(PROGRAM, vec![key(0); self.count])],
            AccessListPattern::DistinctKeys => {
                vec![item(PROGRAM, (0..self.count).map(key).collect())]
            }
            AccessListPattern::RepeatedItems => vec![item(PROGRAM, vec![key(0)]); self.count],
            AccessListPattern::ReversedDistinctKeys => {
                vec![item(PROGRAM, (0..self.count).rev().map(key).collect())]
            }
        };
        Ok(AccessList(list))
    }

    /// Validates the actual included signed envelope, then counts its decoded raw list.
    pub(super) fn validate_transaction(
        &self,
        transaction: &TransactionSigned,
        signer: Address,
        block_index: usize,
    ) -> Result<AccessListBlockReport> {
        let encoded = transaction.encoded_2718();
        let mut bytes = encoded.as_slice();
        let decoded = TransactionSigned::decode_2718(&mut bytes)?;
        ensure!(
            bytes.is_empty() && decoded == *transaction,
            "access-list transaction is not canonical EIP-2718"
        );
        ensure!(
            decoded.recover_signer()? == signer && decoded.nonce() == u64::try_from(block_index)?,
            "included access-list signer or nonce differs"
        );
        let transaction_type = if decoded.as_eip7702().is_some() {
            4
        } else {
            ensure!(
                decoded.as_eip1559().is_some(),
                "access-list fixture requires type 2 or type 4"
            );
            2
        };
        let list = decoded
            .access_list()
            .ok_or_else(|| anyhow::anyhow!("included transaction has no access list"))?;
        ensure!(
            *list == self.canonical_list()?,
            "included raw access-list order or entries differ"
        );
        let mut addresses = BTreeSet::new();
        let mut pairs = BTreeSet::new();
        let mut raw_storage_keys = 0_usize;
        for item in &list.0 {
            addresses.insert(item.address);
            raw_storage_keys = raw_storage_keys
                .checked_add(item.storage_keys.len())
                .ok_or_else(|| anyhow::anyhow!("access-list key count overflow"))?;
            for key in &item.storage_keys {
                pairs.insert((item.address, *key));
            }
        }
        Ok(AccessListBlockReport {
            block_index,
            transaction_type,
            raw_address_items: list.0.len(),
            raw_storage_keys,
            unique_addresses: addresses.len(),
            unique_storage_pairs: pairs.len(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_consensus::{SignableTransaction, TxEip1559};
    use alloy_signer::SignerSync;
    use alloy_signer_local_fixture::PrivateKeySigner;

    #[test]
    fn included_access_list_gate_rejects_changed_list_signer_and_nonce() {
        let descriptor = ProgramAccessList {
            pattern: AccessListPattern::DistinctKeys,
            count: 4,
        };
        let signer = PrivateKeySigner::from_bytes(&B256::repeat_byte(0x11)).unwrap();
        let sign = |list| {
            let tx = TxEip1559 {
                nonce: 0,
                chain_id: 167_000,
                gas_limit: 15_000_000,
                access_list: list,
                ..Default::default()
            };
            let signature = signer.sign_hash_sync(&tx.signature_hash()).unwrap();
            TransactionSigned::from(tx.into_signed(signature))
        };
        let list = descriptor.canonical_list().unwrap();
        let original = sign(list.clone());
        assert!(
            descriptor
                .validate_transaction(&original, signer.address(), 0)
                .is_ok()
        );
        assert!(
            descriptor
                .validate_transaction(&original, Address::ZERO, 0)
                .is_err()
        );
        assert!(
            descriptor
                .validate_transaction(&original, signer.address(), 1)
                .is_err()
        );
        let mut reversed = list;
        reversed.0[0].storage_keys.reverse();
        assert!(
            descriptor
                .validate_transaction(&sign(reversed), signer.address(), 0)
                .is_err()
        );
        assert!(
            descriptor
                .validate_transaction(&sign(AccessList::default()), signer.address(), 0)
                .is_err()
        );
    }
}
