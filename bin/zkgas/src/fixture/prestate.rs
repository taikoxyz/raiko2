//! Host-only predeployed contracts and proofs selected by authenticated native reads.
use super::{
    Program, constructor::ControlledTrieState, observer::NativeReads, transactions::TransactionKind,
};
use alloy_primitives::{Address, B256, Bytes, keccak256};
use alloy_trie::{
    HashBuilder, Nibbles,
    proof::{AddedRemovedKeys, ProofRetainer},
};
use anyhow::{Result, ensure};
use raiko2_primitives::{ExecutionWitness, WitnessStateNode};
use reth_trie_common::HashedPostState;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrestateContract {
    pub address: String,
    pub code_hex: String,
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

pub(super) fn validate(program: &Program) -> Result<()> {
    let Some(contracts) = &program.prestate_contracts else {
        return Ok(());
    };
    ensure!(
        (1..=512).contains(&contracts.len()),
        "prestate_contracts requires1..=512 contracts"
    );
    ensure!(
        program.block_count == 1
            && program
                .transactions
                .as_ref()
                .is_some_and(|t| t.kind == TransactionKind::Call),
        "prestate_contracts requires one block and explicit CALL transactions"
    );
    ensure!(
        program.storage_state.is_none()
            && program.access_list.is_none()
            && program.authorizations.is_none(),
        "prestate_contracts cannot combine storage_state, access_list or authorizations"
    );
    let mut addresses = BTreeSet::new();
    let reserved = super::constructor::prestate_reserved_addresses()?;
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
}
impl PrestateReport {
    pub(super) fn new(
        contracts: &[PrestateContract],
        reads: &NativeReads,
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
        Ok(Self {
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
