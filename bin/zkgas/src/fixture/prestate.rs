//! Host-only predeployed contracts and proofs selected by authenticated native reads.
use super::{
    constructor::ControlledTrieState,
    observer::NativeReads,
    transactions::{ProgramTransactions, TransactionKind},
};
use alloy_primitives::{Address, B256, Bytes, U256, keccak256};
use alloy_trie::{
    HashBuilder, Nibbles,
    proof::{AddedRemovedKeys, ProofRetainer},
};
use anyhow::{Result, ensure};
use raiko2_primitives::{ExecutionWitness, WitnessStateNode};
use reth_trie_common::HashedPostState;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PrestateContract {
    pub address: String,
    pub code_hex: String,
    /// Raw EVM slots, never pre-hashed trie keys. Duplicate numeric slots are rejected.
    #[serde(
        default,
        deserialize_with = "deserialize_initial_storage",
        skip_serializing_if = "Option::is_none"
    )]
    pub initial_storage: Option<BTreeMap<U256, U256>>,
}

fn deserialize_initial_storage<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<BTreeMap<U256, U256>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct Storage;
    impl<'de> serde::de::Visitor<'de> for Storage {
        type Value = BTreeMap<U256, U256>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("a raw-slot storage map without duplicate numeric slots")
        }
        fn visit_map<M: serde::de::MapAccess<'de>>(
            self,
            mut map: M,
        ) -> std::result::Result<Self::Value, M::Error> {
            let mut storage = BTreeMap::new();
            while let Some((slot, value)) = map.next_entry::<U256, U256>()? {
                if storage.insert(slot, value).is_some() {
                    return Err(serde::de::Error::custom(
                        "duplicate numeric initial_storage slot",
                    ));
                }
            }
            Ok(storage)
        }
    }
    // Explicit null, like omission, disables initialized-storage diagnostics.
    struct OptionalStorage;
    impl<'de> serde::de::Visitor<'de> for OptionalStorage {
        type Value = Option<BTreeMap<U256, U256>>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("an optional raw-slot storage map")
        }
        fn visit_none<E: serde::de::Error>(self) -> std::result::Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_some<D: serde::Deserializer<'de>>(
            self,
            deserializer: D,
        ) -> std::result::Result<Self::Value, D::Error> {
            deserializer.deserialize_map(Storage).map(Some)
        }
    }
    deserializer.deserialize_option(OptionalStorage)
}
impl PrestateContract {
    pub(super) fn decoded(&self) -> Result<(Address, Bytes)> {
        ensure!(
            self.address.starts_with("0x") && self.address.len() == 42,
            "prestate contract address requires exactly20 hex bytes with0x prefix"
        );
        let address = self.address.parse::<Address>()?;
        let code = Bytes::from(super::program_bytes(&self.code_hex)?);
        ensure!(
            !code.is_empty() && code.len() <= 24_576,
            "prestate contract requires1..=24576 code bytes"
        );
        // These represent ordinary deployed runtime, not EIP-7702 delegation designators.
        ensure!(
            code[0] != 0xef,
            "prestate contract cannot use reserved0xef runtime prefix"
        );
        Ok((address, code))
    }
}

/// Shared parser and core-constructor validation; neither boundary may bypass these rules.
pub(super) fn validate(
    contracts: Option<&[PrestateContract]>,
    transactions: Option<&ProgramTransactions>,
    block_count: usize,
    storage: bool,
    access_list: bool,
    authorizations: bool,
) -> Result<()> {
    let per_transaction_lists = transactions.is_some_and(|t| t.access_lists.is_some());
    let recipients = transactions.and_then(|t| t.recipients.as_deref());
    if let Some(recipients) = recipients {
        let t = transactions.expect("recipients have a transaction descriptor");
        ensure!(
            t.kind == TransactionKind::Call && block_count == 1 && contracts.is_some(),
            "transactions recipients requires one block, CALL and prestate_contracts"
        );
        ensure!(
            recipients.len() == t.count_per_block,
            "transactions recipients length must equal count_per_block"
        );
    }
    let Some(contracts) = contracts else {
        return Ok(());
    };
    ensure!(
        (1..=512).contains(&contracts.len()),
        "prestate_contracts requires1..=512 contracts"
    );
    ensure!(
        block_count == 1 && transactions.is_some_and(|t| t.kind == TransactionKind::Call),
        "prestate_contracts requires one block and explicit CALL transactions"
    );
    ensure!(
        !storage && !access_list && !authorizations,
        "prestate_contracts cannot combine storage_state, access_list or authorizations"
    );
    ensure!(
        !per_transaction_lists
            || contracts
                .iter()
                .all(|c| c.initial_storage.as_ref().is_none_or(|s| s.is_empty())),
        "per-transaction access_lists prestate cannot initialize storage"
    );
    let mut addresses = BTreeSet::new();
    let reserved = super::constructor::prestate_reserved_addresses()?;
    let mut storage_entries = 0usize;
    for contract in contracts {
        let (address, _) = contract.decoded()?;
        ensure!(
            address.as_slice()[..18].iter().any(|byte| *byte != 0) && !reserved.contains(&address),
            "prestate contract address is reserved"
        );
        ensure!(
            addresses.insert(address),
            "duplicate prestate contract address"
        );
        if let Some(storage) = &contract.initial_storage {
            storage_entries = storage_entries
                .checked_add(storage.len())
                .ok_or_else(|| anyhow::anyhow!("initial_storage entry count overflow"))?;
            ensure!(
                storage_entries <= 16_384,
                "prestate_contracts initial_storage exceeds16384 entries"
            );
            ensure!(
                storage.values().all(|value| !value.is_zero()),
                "initial_storage values must be nonzero"
            );
        }
    }
    if let Some(recipients) = recipients {
        ensure!(
            recipients.iter().all(|address| addresses.contains(address)),
            "transactions recipient must be a declared prestate contract"
        );
    }
    Ok(())
}

#[derive(Debug, Serialize)]
pub(super) struct PrestateReport {
    pub configured_account_count: usize,
    pub configured_unique_code_count: usize,
    /// Sum of byte lengths per configured address, including shared code.
    pub configured_code_bytes: usize,
    pub configured_unique_code_bytes: usize,
    pub witnessed_account_count: usize,
    pub witnessed_unique_code_count: usize,
    pub witnessed_code_bytes: usize,
    pub loaded_account_target_count: usize,
    pub loaded_storage_target_count: usize,
    pub total_witness_unique_code_count: usize,
    pub total_witness_code_bytes: usize,
    pub full_witness_node_count: usize,
    pub minimized_witness_node_count: usize,
    pub witness_policy: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initialized_storage: Option<Vec<InitializedStorageReport>>,
}

#[derive(Debug, Serialize)]
pub(super) struct InitializedStorageReport {
    pub address: Address,
    pub initial_storage_root: B256,
    pub initial_storage_entry_count: usize,
    /// None means the contract is absent in committed block-final state.
    pub final_storage_root: Option<B256>,
    pub final_storage_entry_count: usize,
}
impl PrestateReport {
    pub(super) fn new(
        contracts: &[PrestateContract],
        reads: &NativeReads,
        initial_state: &ControlledTrieState,
        full: usize,
        minimized: usize,
    ) -> Result<Self> {
        let mut configured = BTreeMap::new();
        let mut witnessed = BTreeMap::new();
        let mut accounts = 0;
        let mut configured_bytes = 0;
        for contract in contracts {
            let (address, code) = contract.decoded()?;
            let hash = keccak256(&code);
            configured_bytes += code.len();
            configured.insert(hash, code.clone());
            if reads.accounts.contains(&address) {
                accounts += 1;
            }
            if reads.codes.contains_key(&hash) {
                witnessed.insert(hash, code);
            }
        }
        let initialized_storage = contracts
            .iter()
            .any(|c| c.initial_storage.is_some())
            .then(|| {
                contracts
                    .iter()
                    .map(|contract| {
                        let address = contract.address.parse::<Address>()?;
                        let hash = keccak256(address);
                        let account = initial_state.accounts.get(&hash).ok_or_else(|| {
                            anyhow::anyhow!("configured contract missing from initial state")
                        })?;
                        Ok(InitializedStorageReport {
                            address,
                            initial_storage_root: account.storage_root,
                            initial_storage_entry_count: initial_state
                                .storages
                                .get(&hash)
                                .map_or(0, BTreeMap::len),
                            final_storage_root: None,
                            final_storage_entry_count: 0,
                        })
                    })
                    .collect::<Result<Vec<_>>>()
            })
            .transpose()?;
        Ok(Self {
            initialized_storage,
            configured_account_count: contracts.len(),
            configured_unique_code_count: configured.len(),
            configured_code_bytes: configured_bytes,
            configured_unique_code_bytes: configured.values().map(|code| code.len()).sum(),
            witnessed_account_count: accounts,
            witnessed_unique_code_count: witnessed.len(),
            witnessed_code_bytes: witnessed.values().map(|code| code.len()).sum(),
            loaded_account_target_count: reads.accounts.len(),
            loaded_storage_target_count: reads.storage.values().map(BTreeSet::len).sum(),
            total_witness_unique_code_count: reads.codes.len(),
            total_witness_code_bytes: reads.codes.values().map(|code| code.len()).sum(),
            full_witness_node_count: full,
            minimized_witness_node_count: minimized,
            witness_policy: "native_database_reads_multiproof_and_hash_deduplicated_code",
        })
    }
    /// Called only after witness() refreshes storage roots and the block root agrees.
    pub(super) fn finish_storage(&mut self, state: &ControlledTrieState) {
        if let Some(contracts) = &mut self.initialized_storage {
            for contract in contracts {
                let hash = keccak256(contract.address);
                contract.final_storage_root = state.accounts.get(&hash).map(|a| a.storage_root);
                contract.final_storage_entry_count =
                    state.storages.get(&hash).map_or(0, BTreeMap::len);
            }
        }
    }
}

fn proof<'a>(
    leaves: impl Iterator<Item = (B256, Vec<u8>)> + 'a,
    targets: impl Iterator<Item = B256>,
    mutations: AddedRemovedKeys,
    nodes: &mut Vec<WitnessStateNode>,
) -> B256 {
    let mut builder = HashBuilder::default().with_proof_retainer(
        ProofRetainer::new(targets.map(Nibbles::unpack).collect())
            .with_added_removed_keys(Some(mutations)),
    );
    for (key, value) in leaves {
        builder.add_leaf(Nibbles::unpack(key), &value);
    }
    let root = builder.root();
    nodes.extend(
        builder
            .take_proof_nodes()
            .into_nodes_sorted()
            .into_iter()
            .map(|(_, bytes)| WitnessStateNode::from_bytes(bytes)),
    );
    root
}

pub(super) fn minimized_witness(
    state: &ControlledTrieState,
    reads: &NativeReads,
    post: &HashedPostState,
) -> (B256, Vec<WitnessStateNode>) {
    let mut nodes = Vec::new();
    for (address, slots) in &reads.storage {
        let hash = keccak256(address);
        let mut mutations = AddedRemovedKeys::default();
        let empty = BTreeMap::new();
        let storage = state.storages.get(&hash).unwrap_or(&empty);
        if let Some(changes) = post.storages.get(&hash) {
            for (key, value) in &changes.storage {
                if value.is_zero() && storage.contains_key(key) {
                    mutations.insert_removed(*key);
                }
                if !value.is_zero() && !storage.contains_key(key) {
                    mutations.insert_added(*key);
                }
            }
        }
        proof(
            storage
                .iter()
                .map(|(key, value)| (*key, alloy_rlp::encode(*value))),
            slots.iter().map(|slot| keccak256(slot.to_be_bytes::<32>())),
            mutations,
            &mut nodes,
        );
    }
    let mut mutations = AddedRemovedKeys::default();
    for (key, value) in &post.accounts {
        if value.is_some() && !state.accounts.contains_key(key) {
            mutations.insert_added(*key);
        }
        if value.is_none() && state.accounts.contains_key(key) {
            mutations.insert_removed(*key);
        }
    }
    let targets = reads
        .accounts
        .iter()
        .map(keccak256)
        .chain(post.accounts.keys().copied())
        .collect::<BTreeSet<_>>();
    let root = proof(
        state
            .accounts
            .iter()
            .map(|(key, value)| (*key, alloy_rlp::encode(value))),
        targets.into_iter(),
        mutations,
        &mut nodes,
    );
    (root, ExecutionWitness::canonicalize_state_nodes(nodes))
}
