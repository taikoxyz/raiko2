//! Raw EVM storage prestate and target multiproofs for local one-block experiments.

use std::collections::{BTreeMap, BTreeSet};

use alloy_consensus::Transaction as _;
use alloy_primitives::{Address, B256, U256, keccak256};
use alloy_trie::{
    HashBuilder, Nibbles,
    proof::{AddedRemovedKeys, ProofRetainer},
};
use anyhow::{Context as _, Result, bail};
use raiko2_primitives::{ExecutionWitness, WitnessStateNode};
use serde::{Deserialize, Serialize};

use super::constructor::{ControlledMptFixture, ControlledTrieState};
use reth_ethereum_primitives::TransactionSigned;

/// Producer-only raw slot preimages. This never changes the serialized GuestInput contract.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramStorageState {
    /// Existing nonzero values keyed by the original EVM storage slot, not its hash.
    pub initial_storage: BTreeMap<U256, U256>,
    /// All storage slots the reviewed program may read or write, including absent keys.
    pub proof_slots: Vec<U256>,
    #[serde(default)]
    pub insert_slots: Vec<U256>,
    #[serde(default)]
    pub delete_slots: Vec<U256>,
}

/// Construction facts, not a claim that an arbitrary program uses every supplied proof target.
#[derive(Clone, Debug, Serialize)]
pub struct ProgramStorageStateReport {
    pub initial_storage_entry_count: usize,
    pub proof_slot_count: usize,
    pub insert_slot_count: usize,
    pub delete_slot_count: usize,
    pub full_witness_node_count: usize,
    pub minimized_witness_node_count: usize,
    pub witness_policy: &'static str,
}

impl ProgramStorageState {
    pub fn validate(&self, block_count: usize) -> Result<()> {
        if block_count != 1 {
            bail!("storage_state supports exactly one block");
        }
        if self.initial_storage.len() > 16_384
            || self.initial_storage.values().any(U256::is_zero)
            || self.proof_slots.is_empty()
            || self.proof_slots.len() > 8_192
        {
            bail!("storage_state requires at most16384 nonzero entries and1..=8192 proof slots");
        }
        let proof = unique_slots(&self.proof_slots, "proof_slots")?;
        let inserted = unique_slots(&self.insert_slots, "insert_slots")?;
        let deleted = unique_slots(&self.delete_slots, "delete_slots")?;
        if !inserted.is_subset(&proof)
            || !deleted.is_subset(&proof)
            || !inserted.is_disjoint(&deleted)
            || inserted
                .iter()
                .any(|slot| self.initial_storage.contains_key(slot))
            || deleted
                .iter()
                .any(|slot| !self.initial_storage.contains_key(slot))
        {
            bail!("storage_state mutation slots must match proof targets and prestate presence");
        }
        Ok(())
    }

    pub(super) fn report(&self, full: usize, minimized: usize) -> ProgramStorageStateReport {
        ProgramStorageStateReport {
            initial_storage_entry_count: self.initial_storage.len(),
            proof_slot_count: self.proof_slots.len(),
            insert_slot_count: self.insert_slots.len(),
            delete_slot_count: self.delete_slots.len(),
            full_witness_node_count: full,
            minimized_witness_node_count: minimized,
            witness_policy: "target_storage_multiproof_with_complete_system_storage",
        }
    }
}

fn unique_slots(slots: &[U256], name: &str) -> Result<BTreeSet<U256>> {
    let keys = slots.iter().copied().collect::<BTreeSet<_>>();
    if keys.len() != slots.len() {
        bail!("storage_state {name} contains duplicate raw slots");
    }
    Ok(keys)
}

fn hashed_slot(slot: U256) -> B256 {
    keccak256(slot.to_be_bytes::<32>())
}

fn retain_storage_nodes(
    storage: &BTreeMap<B256, U256>,
    targets: Vec<Nibbles>,
    mutations: AddedRemovedKeys,
    nodes: &mut Vec<WitnessStateNode>,
) {
    let retainer = ProofRetainer::new(targets).with_added_removed_keys(Some(mutations));
    let mut builder = HashBuilder::default().with_proof_retainer(retainer);
    for (key, value) in storage {
        builder.add_leaf(Nibbles::unpack(*key), &alloy_rlp::encode(*value));
    }
    let _ = builder.root();
    nodes.extend(
        builder
            .take_proof_nodes()
            .into_nodes_sorted()
            .into_iter()
            .map(|(_, bytes)| WitnessStateNode::from_bytes(bytes)),
    );
}

impl ControlledMptFixture for ProgramStorageState {
    fn apply_fixture(
        &self,
        state: &mut ControlledTrieState,
        candidates: &[TransactionSigned],
    ) -> Result<()> {
        self.validate(1)?;
        let recipient = candidates
            .first()
            .and_then(TransactionSigned::to)
            .context("storage_state candidate must be a call")?;
        let storage = state.storages.entry(keccak256(recipient)).or_default();
        storage.extend(
            self.initial_storage
                .iter()
                .map(|(slot, value)| (hashed_slot(*slot), *value)),
        );
        Ok(())
    }

    fn minimized_fixture_witness(
        &self,
        state: &ControlledTrieState,
        candidates: &[TransactionSigned],
    ) -> Result<(B256, Vec<WitnessStateNode>)> {
        let recipient = candidates
            .first()
            .and_then(TransactionSigned::to)
            .context("storage_state candidate must be a call")?;
        let target = keccak256(recipient);
        let mut nodes = Vec::new();
        for (address, storage) in &state.storages {
            let mut mutations = AddedRemovedKeys::default();
            let targets = if *address == target {
                for slot in &self.insert_slots {
                    mutations.insert_added(hashed_slot(*slot));
                }
                for slot in &self.delete_slots {
                    mutations.insert_removed(hashed_slot(*slot));
                }
                self.proof_slots
                    .iter()
                    .map(|slot| Nibbles::unpack(hashed_slot(*slot)))
                    .collect()
            } else {
                // The local system/Anchor storage is small and must remain available in full.
                storage.keys().copied().map(Nibbles::unpack).collect()
            };
            retain_storage_nodes(storage, targets, mutations, &mut nodes);
        }

        // Match the existing fixture's account proof, including an absent fee-recipient insertion.
        let fee_recipient = keccak256(Address::from([
            0x00, 0x00, 0x77, 0x77, 0x35, 0x36, 0x7b, 0x36, 0xbc, 0x9b, 0x61, 0xc5, 0x00, 0x22,
            0xd9, 0xd0, 0x70, 0x0d, 0xb4, 0xec,
        ]));
        let mut targets = state
            .accounts
            .keys()
            .copied()
            .map(Nibbles::unpack)
            .collect::<Vec<_>>();
        targets.push(Nibbles::unpack(fee_recipient));
        let mut mutations = AddedRemovedKeys::default();
        mutations.insert_added(fee_recipient);
        let retainer = ProofRetainer::new(targets).with_added_removed_keys(Some(mutations));
        let mut builder = HashBuilder::default().with_proof_retainer(retainer);
        for (key, account) in &state.accounts {
            builder.add_leaf(Nibbles::unpack(*key), &alloy_rlp::encode(account));
        }
        let root = builder.root();
        nodes.extend(
            builder
                .take_proof_nodes()
                .into_nodes_sorted()
                .into_iter()
                .map(|(_, bytes)| WitnessStateNode::from_bytes(bytes)),
        );
        Ok((root, ExecutionWitness::canonicalize_state_nodes(nodes)))
    }
}
