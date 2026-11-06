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
    /// One PROGRAM item with an explicit raw key vector, including ordered duplicates.
    RawKeys,
    /// Explicit ordered owner items with empty key lists; duplicates are retained.
    RawAddresses,
}

/// Optional bounded access-list description for an otherwise unchanged user transaction.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramAccessList {
    /// Requested raw vector shape.
    pub pattern: AccessListPattern,
    /// Pattern cardinality, in raw address items or storage keys; 1..=8192.
    pub count: usize,
    /// Exact raw keys for `RawKeys` only; omitted for every existing pattern.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_keys: Option<Vec<B256>>,
    /// Exact empty-key owners for `RawAddresses` only; omitted for all older patterns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub addresses: Option<Vec<Address>>,
}

/// Requested vector shape and independent observations from included signed transactions.
#[derive(Debug, Serialize)]
pub struct AccessListReport {
    /// Requested raw vector shape.
    pub pattern: AccessListPattern,
    /// Legacy name: requested vector cardinality in each included user transaction.
    pub count_per_block: usize,
    /// Present only for multiple users sharing the same descriptor in each block.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transactions_per_block: Option<usize>,
    /// Included user-transaction observations in block order.
    pub blocks: Vec<AccessListBlockReport>,
}

/// One record per block, preserving the legacy single-user serialized shape.
#[derive(Debug, Serialize)]
pub struct AccessListBlockReport {
    pub block_index: usize,
    /// Raw counts are sums over included users; unique counts are block unions.
    /// Every user has the same checked list, so its unique set is already the union.
    #[serde(flatten)]
    pub counts: AccessListCounts,
    /// Explicit per-user counts only for multiple users; no cross-TX deduplication.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transactions: Option<Vec<AccessListTransactionReport>>,
}

#[derive(Debug, Serialize)]
pub struct AccessListTransactionReport {
    /// Included index in the block; Anchor occupies index zero.
    pub transaction_index: usize,
    pub nonce: u64,
    pub transaction_hash: B256,
    #[serde(flatten)]
    pub counts: AccessListCounts,
}

/// Selectors from one canonical decoded signed raw list, without inferred state loads.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AccessListCounts {
    pub transaction_type: u8,
    pub raw_address_items: usize,
    pub raw_storage_keys: usize,
    pub unique_addresses: usize,
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
        ensure!(
            self.pattern == AccessListPattern::RawAddresses || self.addresses.is_none(),
            "addresses is only valid with raw_addresses"
        );
        match self.pattern {
            AccessListPattern::RawAddresses => {
                ensure!(
                    self.storage_keys.is_none(),
                    "raw_addresses cannot include storage_keys"
                );
                ensure!(
                    self.addresses
                        .as_ref()
                        .is_some_and(|addresses| addresses.len() == self.count),
                    "raw access-list addresses length must equal count"
                );
            }
            AccessListPattern::RawKeys => ensure!(
                self.storage_keys
                    .as_ref()
                    .is_some_and(|keys| keys.len() == self.count),
                "raw access-list storage_keys length must equal count"
            ),
            _ => ensure!(
                self.storage_keys.is_none(),
                "storage_keys is only valid with raw_keys"
            ),
        }
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
            AccessListPattern::RawAddresses => self
                .addresses
                .as_ref()
                .expect("validated raw address vector")
                .iter()
                .map(|&address| item(address, vec![]))
                .collect(),
            AccessListPattern::RawKeys => {
                vec![item(
                    PROGRAM,
                    self.storage_keys.clone().expect("validated raw key vector"),
                )]
            }
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
        expected_nonce: u64,
    ) -> Result<AccessListBlockReport> {
        let encoded = transaction.encoded_2718();
        let mut bytes = encoded.as_slice();
        let decoded = TransactionSigned::decode_2718(&mut bytes)?;
        ensure!(
            bytes.is_empty() && decoded == *transaction,
            "access-list transaction is not canonical EIP-2718"
        );
        ensure!(
            decoded.recover_signer()? == signer && decoded.nonce() == expected_nonce,
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
            counts: AccessListCounts {
                transaction_type,
                raw_address_items: list.0.len(),
                raw_storage_keys,
                unique_addresses: addresses.len(),
                unique_storage_pairs: pairs.len(),
            },
            transactions: None,
        })
    }

    /// Gate every included user, retaining one block report and exact per-TX selectors.
    pub(super) fn validate_block(
        &self,
        transactions: &[TransactionSigned],
        signer: Address,
        block_index: usize,
        users_per_block: usize,
    ) -> Result<AccessListBlockReport> {
        ensure!(
            users_per_block > 0 && transactions.len() == users_per_block,
            "included access-list user count differs"
        );
        let first_nonce = block_index
            .checked_mul(users_per_block)
            .ok_or_else(|| anyhow::anyhow!("access-list user nonce overflow"))?;
        let mut users = Vec::with_capacity(users_per_block);
        for (index, transaction) in transactions.iter().enumerate() {
            let nonce = first_nonce
                .checked_add(index)
                .ok_or_else(|| anyhow::anyhow!("access-list user nonce overflow"))?;
            let report =
                self.validate_transaction(transaction, signer, block_index, u64::try_from(nonce)?)?;
            users.push(AccessListTransactionReport {
                transaction_index: index + 1,
                nonce: transaction.nonce(),
                transaction_hash: *transaction.hash(),
                counts: report.counts,
            });
        }
        let mut counts = users[0].counts.clone();
        ensure!(
            users.iter().all(|user| user.counts == counts),
            "shared access-list user selectors differ"
        );
        // Each raw vector is validated against the same exact canonical descriptor.
        // Therefore the block union equals one user's unique set, without another set.
        counts.raw_address_items = counts
            .raw_address_items
            .checked_mul(users_per_block)
            .ok_or_else(|| anyhow::anyhow!("access-list raw address count overflow"))?;
        counts.raw_storage_keys = counts
            .raw_storage_keys
            .checked_mul(users_per_block)
            .ok_or_else(|| anyhow::anyhow!("access-list raw key count overflow"))?;
        Ok(AccessListBlockReport {
            block_index,
            counts,
            transactions: (users_per_block > 1).then_some(users),
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
    fn raw_addresses_json_preserves_empty_key_items_order_and_duplicates() {
        let addresses = vec![Address::ZERO, Address::repeat_byte(0xff), Address::ZERO];
        let value = serde_json::json!({
            "pattern": "raw_addresses", "count": 3, "addresses": addresses,
        });
        let ordered_json = format!(
            r#"{{"pattern":"raw_addresses","count":3,"addresses":{}}}"#,
            serde_json::to_string(&addresses).unwrap()
        );
        let descriptor: ProgramAccessList = serde_json::from_str(&ordered_json)
            .expect("explicit raw_addresses descriptor must parse");
        let list = descriptor.canonical_list().unwrap();
        assert_eq!(
            list.0.iter().map(|item| item.address).collect::<Vec<_>>(),
            addresses
        );
        assert!(list.0.iter().all(|item| item.storage_keys.is_empty()));
        assert_eq!(serde_json::to_value(&descriptor).unwrap(), value);
    }

    #[test]
    fn raw_addresses_enforces_vector_domain_and_field_exclusivity() {
        for value in [
            serde_json::json!({"pattern":"raw_addresses", "count":1}),
            serde_json::json!({"pattern":"raw_addresses", "count":1, "addresses":null}),
            serde_json::json!({"pattern":"raw_addresses", "count":1, "addresses":[]}),
            serde_json::json!({"pattern":"raw_addresses", "count":2, "addresses":[Address::ZERO]}),
            serde_json::json!({"pattern":"raw_addresses", "count":0, "addresses":[]}),
            serde_json::json!({"pattern":"raw_addresses", "count":1, "addresses":[Address::ZERO], "storage_keys":[]}),
        ] {
            let descriptor: ProgramAccessList = serde_json::from_value(value).unwrap();
            assert!(descriptor.validate().is_err());
        }
        for value in [
            serde_json::json!({"pattern":"raw_addresses", "count":1, "addresses":[true]}),
            serde_json::json!({"pattern":"raw_addresses", "count":1, "addresses":[0]}),
            serde_json::json!({"pattern":"raw_addresses", "count":1, "addresses":["0x00"]}),
            serde_json::json!({"pattern":"raw_addresses", "count":1, "addresses":[format!("0x{}", "00".repeat(21))]}),
            serde_json::json!({"pattern":"raw_addresses", "count":1, "addresses":["0xgg"]}),
            serde_json::json!({"pattern":"raw_addresses", "count":1, "addresses":{}}),
            serde_json::json!({"pattern":"raw_addresses", "count":true, "addresses":[Address::ZERO]}),
            serde_json::json!({"pattern":"raw_addresses", "count":1.0, "addresses":[Address::ZERO]}),
            serde_json::json!({"pattern":"raw_addresses", "count":-1, "addresses":[Address::ZERO]}),
            serde_json::json!({"pattern":"raw_addresses", "count":1, "addresses":[Address::ZERO], "unknown":true}),
        ] {
            assert!(serde_json::from_value::<ProgramAccessList>(value).is_err());
        }
        for count in [1, 8192, 8193] {
            let descriptor: ProgramAccessList = serde_json::from_value(serde_json::json!({
                "pattern":"raw_addresses", "count":count, "addresses":vec![Address::ZERO; count],
            }))
            .unwrap();
            assert_eq!(descriptor.canonical_list().is_ok(), count <= 8192);
        }
        for pattern in [
            "repeated_address",
            "distinct_addresses",
            "repeated_key",
            "distinct_keys",
            "repeated_items",
            "reversed_distinct_keys",
            "raw_keys",
        ] {
            let mut value = serde_json::json!({"pattern":pattern, "count":1});
            if pattern == "raw_keys" {
                value["storage_keys"] = serde_json::json!([B256::ZERO]);
            }
            let original = value.clone();
            value["addresses"] = serde_json::Value::Null;
            let descriptor: ProgramAccessList = serde_json::from_value(value.clone()).unwrap();
            assert!(descriptor.validate().is_ok());
            assert_eq!(serde_json::to_value(descriptor).unwrap(), original);
            for addresses in [serde_json::json!([]), serde_json::json!([Address::ZERO])] {
                value["addresses"] = addresses;
                let descriptor: ProgramAccessList = serde_json::from_value(value.clone()).unwrap();
                assert!(descriptor.validate().is_err());
            }
        }
    }

    #[test]
    fn raw_addresses_signed_multi_user_gate_checks_every_raw_item_and_nonce() {
        let descriptor: ProgramAccessList = serde_json::from_value(serde_json::json!({
            "pattern":"raw_addresses", "count":4,
            "addresses":[Address::ZERO, Address::repeat_byte(0xff), Address::ZERO, Address::with_last_byte(1)],
        })).unwrap();
        let signer = PrivateKeySigner::from_bytes(&B256::repeat_byte(0x11)).unwrap();
        let sign = |nonce, list, signer: &PrivateKeySigner| {
            let tx = TxEip1559 {
                nonce,
                chain_id: 167_000,
                gas_limit: 15_000_000,
                access_list: list,
                ..Default::default()
            };
            let signature = signer.sign_hash_sync(&tx.signature_hash()).unwrap();
            TransactionSigned::from(tx.into_signed(signature))
        };
        let list = descriptor.canonical_list().unwrap();
        let users = vec![
            sign(2, list.clone(), &signer),
            sign(3, list.clone(), &signer),
        ];
        let report = descriptor
            .validate_block(&users, signer.address(), 1, 2)
            .unwrap();
        assert_eq!(
            (
                report.counts.raw_address_items,
                report.counts.raw_storage_keys,
                report.counts.unique_addresses,
                report.counts.unique_storage_pairs
            ),
            (8, 0, 3, 0)
        );
        let nested = report.transactions.unwrap();
        assert!(
            nested
                .iter()
                .all(|row| row.counts.raw_address_items == 4 && row.counts.unique_addresses == 3)
        );
        assert_eq!(
            nested.iter().map(|row| row.nonce).collect::<Vec<_>>(),
            vec![2, 3]
        );
        let mut reversed = list.clone();
        reversed.0.reverse();
        let mut removed_duplicate = list.clone();
        removed_duplicate.0.remove(2);
        let mut added_key = list.clone();
        added_key.0[1].storage_keys.push(B256::ZERO);
        let other_signer = PrivateKeySigner::from_bytes(&B256::repeat_byte(0x22)).unwrap();
        for second in [
            sign(3, reversed, &signer),
            sign(3, removed_duplicate, &signer),
            sign(3, added_key, &signer),
            sign(2, list.clone(), &signer),
            sign(3, list.clone(), &other_signer),
        ] {
            assert!(
                descriptor
                    .validate_block(&[users[0].clone(), second], signer.address(), 1, 2)
                    .is_err()
            );
        }
        let single = descriptor
            .validate_transaction(&users[0], signer.address(), 1, 2)
            .unwrap();
        assert!(single.transactions.is_none());
        assert_eq!(single.counts.raw_address_items, 4);
    }

    #[test]
    fn raw_keys_json_accepts_exact_order_and_duplicate_zero_extreme_keys() {
        let keys = vec![B256::ZERO, B256::repeat_byte(0xff), B256::ZERO];
        let descriptor: ProgramAccessList = serde_json::from_value(serde_json::json!({
            "pattern": "raw_keys", "count": 3, "storage_keys": keys,
        }))
        .expect("explicit raw_keys descriptor must parse");
        let list = descriptor.canonical_list().unwrap();
        assert_eq!(list.0.len(), 1);
        assert_eq!(list.0[0].address, PROGRAM);
        assert_eq!(list.0[0].storage_keys, keys);
    }

    #[test]
    fn old_descriptors_serialize_without_optional_keys_and_reject_raw_vectors() {
        for pattern in [
            "repeated_address",
            "distinct_addresses",
            "repeated_key",
            "distinct_keys",
            "repeated_items",
            "reversed_distinct_keys",
        ] {
            let value = serde_json::json!({"pattern": pattern, "count": 4});
            let descriptor: ProgramAccessList = serde_json::from_value(value.clone()).unwrap();
            assert!(descriptor.validate().is_ok());
            assert_eq!(serde_json::to_value(&descriptor).unwrap(), value);
            for keys in [vec![], vec![B256::ZERO]] {
                let invalid: ProgramAccessList = serde_json::from_value(serde_json::json!({
                    "pattern": pattern, "count": 4, "storage_keys": keys,
                }))
                .unwrap();
                assert!(invalid.validate().is_err());
            }
        }
    }

    #[test]
    fn raw_keys_reject_missing_mismatched_untyped_and_out_of_bounds_inputs() {
        for value in [
            serde_json::json!({"pattern": "raw_keys", "count": 1}),
            serde_json::json!({"pattern": "raw_keys", "count": 1, "storage_keys": null}),
            serde_json::json!({"pattern": "raw_keys", "count": 1, "storage_keys": []}),
            serde_json::json!({"pattern": "raw_keys", "count": 2, "storage_keys": [B256::ZERO]}),
            serde_json::json!({"pattern": "raw_keys", "count": 0, "storage_keys": []}),
        ] {
            let descriptor: ProgramAccessList = serde_json::from_value(value).unwrap();
            assert!(descriptor.validate().is_err());
        }
        for key in [
            serde_json::json!("0x00"),
            serde_json::json!("0xgg"),
            serde_json::json!(format!("0x{}", "00".repeat(33))),
            serde_json::json!(0),
            serde_json::json!(true),
        ] {
            assert!(
                serde_json::from_value::<ProgramAccessList>(serde_json::json!({
                    "pattern": "raw_keys", "count": 1, "storage_keys": [key],
                }))
                .is_err()
            );
        }
        for count in [
            serde_json::json!(true),
            serde_json::json!(1.0),
            serde_json::json!(-1),
            serde_json::json!("1"),
        ] {
            assert!(
                serde_json::from_value::<ProgramAccessList>(serde_json::json!({
                    "pattern": "raw_keys", "count": count, "storage_keys": [B256::ZERO],
                }))
                .is_err()
            );
        }
        for count in [1, 8192, 8193] {
            let descriptor = ProgramAccessList {
                pattern: AccessListPattern::RawKeys,
                count,
                storage_keys: Some(vec![B256::ZERO; count]),
                addresses: None,
            };
            assert_eq!(descriptor.canonical_list().is_ok(), count <= 8192);
        }
        assert!(
            serde_json::from_value::<ProgramAccessList>(serde_json::json!({
                "pattern": "raw_keys", "count": 1, "storage_keys": [B256::ZERO], "extra": true,
            }))
            .is_err()
        );
    }

    #[test]
    fn raw_signed_list_gate_keeps_order_duplicates_and_actual_unique_counts() {
        let descriptor: ProgramAccessList = serde_json::from_value(serde_json::json!({
            "pattern": "raw_keys", "count": 4,
            "storage_keys": [B256::ZERO, B256::repeat_byte(0xff), B256::ZERO, B256::with_last_byte(1)],
        })).unwrap();
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
        let report = descriptor
            .validate_transaction(&sign(list.clone()), signer.address(), 0, 0)
            .unwrap();
        assert_eq!(
            (
                report.counts.raw_address_items,
                report.counts.raw_storage_keys,
                report.counts.unique_addresses,
                report.counts.unique_storage_pairs
            ),
            (1, 4, 1, 3)
        );
        assert!(
            descriptor
                .validate_transaction(&sign(list.clone()), Address::ZERO, 0, 0)
                .is_err()
        );
        assert!(
            descriptor
                .validate_transaction(&sign(list.clone()), signer.address(), 1, 1)
                .is_err()
        );
        let mut reordered = list.clone();
        reordered.0[0].storage_keys.reverse();
        assert!(
            descriptor
                .validate_transaction(&sign(reordered), signer.address(), 0, 0)
                .is_err()
        );
        let mut missing_duplicate = list;
        missing_duplicate.0[0].storage_keys.remove(2);
        assert!(
            descriptor
                .validate_transaction(&sign(missing_duplicate), signer.address(), 0, 0)
                .is_err()
        );
    }

    #[test]
    fn included_access_list_gate_rejects_changed_list_signer_and_nonce() {
        let descriptor = ProgramAccessList {
            pattern: AccessListPattern::DistinctKeys,
            count: 4,
            storage_keys: None,
            addresses: None,
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
                .validate_transaction(&original, signer.address(), 0, 0)
                .is_ok()
        );
        assert!(
            descriptor
                .validate_transaction(&original, Address::ZERO, 0, 0)
                .is_err()
        );
        assert!(
            descriptor
                .validate_transaction(&original, signer.address(), 1, 1)
                .is_err()
        );
        let mut reversed = list;
        reversed.0[0].storage_keys.reverse();
        assert!(
            descriptor
                .validate_transaction(&sign(reversed), signer.address(), 0, 0)
                .is_err()
        );
        assert!(
            descriptor
                .validate_transaction(&sign(AccessList::default()), signer.address(), 0, 0)
                .is_err()
        );
    }
    #[test]
    fn shared_list_block_gates_every_user_and_global_nonce_without_deduping_raw_counts() {
        let descriptor = ProgramAccessList {
            pattern: AccessListPattern::RawKeys,
            count: 4,
            storage_keys: Some(vec![
                B256::ZERO,
                B256::repeat_byte(0xff),
                B256::ZERO,
                B256::from(U256::from(1)),
            ]),
            addresses: None,
        };
        let signer = PrivateKeySigner::from_bytes(&B256::repeat_byte(0x11)).unwrap();
        let sign = |nonce, list, key: &PrivateKeySigner| {
            let tx = TxEip1559 {
                nonce,
                chain_id: 167_000,
                gas_limit: 15_000_000,
                access_list: list,
                ..Default::default()
            };
            let signature = key.sign_hash_sync(&tx.signature_hash()).unwrap();
            TransactionSigned::from(tx.into_signed(signature))
        };
        let list = descriptor.canonical_list().unwrap();
        for block_index in 0..2 {
            let first = (block_index * 2) as u64;
            let users = vec![
                sign(first, list.clone(), &signer),
                sign(first + 1, list.clone(), &signer),
            ];
            let report = descriptor
                .validate_block(&users, signer.address(), block_index, 2)
                .unwrap();
            assert_eq!(
                (
                    report.counts.raw_address_items,
                    report.counts.raw_storage_keys
                ),
                (2, 8)
            );
            // Cross-transaction union is one identical list, not per-user unique sums.
            assert_eq!(
                (
                    report.counts.unique_addresses,
                    report.counts.unique_storage_pairs
                ),
                (1, 3)
            );
            let nested = report.transactions.unwrap();
            assert_eq!(nested.len(), 2);
            for (index, row) in nested.iter().enumerate() {
                assert_eq!(
                    (row.transaction_index, row.nonce),
                    (index + 1, first + index as u64)
                );
                assert_eq!(row.transaction_hash, *users[index].hash());
                assert_eq!(
                    (row.counts.raw_storage_keys, row.counts.unique_storage_pairs),
                    (4, 3)
                );
            }
            let mut changed = list.clone();
            changed.0[0].storage_keys.reverse();
            let wrong_signer = PrivateKeySigner::from_bytes(&B256::repeat_byte(0x22)).unwrap();
            for second in [
                sign(first + 1, changed, &signer),
                sign(first, list.clone(), &signer),
                sign(first + 1, list.clone(), &wrong_signer),
            ] {
                assert!(
                    descriptor
                        .validate_block(
                            &[users[0].clone(), second],
                            signer.address(),
                            block_index,
                            2
                        )
                        .is_err()
                );
            }
            assert!(
                descriptor
                    .validate_block(&users[..1], signer.address(), block_index, 2)
                    .is_err()
            );
            assert!(
                descriptor
                    .validate_block(&users, signer.address(), block_index, 1)
                    .is_err()
            );
        }
        let report = descriptor
            .validate_block(&[sign(0, list, &signer)], signer.address(), 0, 1)
            .unwrap();
        let json = serde_json::to_value(report).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"block_index":0,"transaction_type":2,
            "raw_address_items":1,"raw_storage_keys":4,"unique_addresses":1,"unique_storage_pairs":3})
        );
        assert!(
            descriptor
                .validate_block(&[], signer.address(), 0, 0)
                .is_err()
        );
    }
}
