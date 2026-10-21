use alloy_consensus::TrieAccount;
#[cfg(feature = "mpt-metrics")]
use alloy_primitives::map::FbBuildHasher;
use alloy_primitives::{Address, B256, Bytes, KECCAK256_EMPTY, U256, keccak256, map::B256Map};
use alloy_trie::EMPTY_ROOT_HASH;
use raiko2_primitives::{ExecutionWitness, StatelessValidationError, WitnessStateNode};
use reth_errors::ProviderError;
use reth_revm::state::Bytecode;
use reth_trie_common::HashedPostState;
use risc0_ethereum_trie::CachedTrie;
#[cfg(feature = "mpt-metrics")]
use std::collections::BTreeSet;
use std::{
    cell::RefCell,
    collections::hash_map::Entry,
    marker::PhantomData,
    panic::{AssertUnwindSafe, catch_unwind},
};

#[cfg(feature = "mpt-metrics")]
use crate::mpt_metrics::{
    HashObservation, MptHydrationMetrics, MptMaterializationMetrics, MptObservationError,
    MptWorkloadMetrics, MutationObservation, PathObservation, TrieSnapshot, observe_mutation,
};
use crate::trie::StatelessTrie;

/// Zero-overhead helper for tries that only contain RLP encoded data.
#[derive(Debug, Clone, Default)]
#[cfg_attr(not(feature = "mpt-metrics"), repr(transparent))]
struct RlpTrie<T> {
    inner: CachedTrie,
    phantom: PhantomData<T>,
    #[cfg(feature = "mpt-metrics")]
    cached_paths: BTreeSet<Vec<u8>>,
    #[cfg(feature = "mpt-metrics")]
    materialization: MptMaterializationMetrics,
    #[cfg(feature = "mpt-metrics")]
    snapshot: TrieSnapshot,
    #[cfg(all(feature = "mpt-metrics", test))]
    snapshot_rebuilds: u64,
}

impl<T: alloy_rlp::Decodable + alloy_rlp::Encodable> RlpTrie<T> {
    #[cfg(not(feature = "mpt-metrics"))]
    const fn new(inner: CachedTrie) -> Self {
        Self {
            inner,
            phantom: PhantomData,
        }
    }

    #[cfg(all(feature = "mpt-metrics", test))]
    fn new(inner: CachedTrie) -> Self {
        TrieSnapshot::ensure_node_bound(inner.size()).expect("test trie exceeds snapshot bound");
        let snapshot = TrieSnapshot::from_rlp_nodes(&inner.rlp_nodes())
            .expect("test trie must have valid RLP nodes");
        Self {
            inner,
            phantom: PhantomData,
            cached_paths: BTreeSet::new(),
            materialization: MptMaterializationMetrics::default(),
            snapshot,
            snapshot_rebuilds: 1,
        }
    }

    fn from_prehashed(
        root: B256,
        rlp_by_digest: &B256Map<impl AsRef<[u8]>>,
    ) -> alloy_rlp::Result<Self> {
        #[cfg(feature = "mpt-metrics")]
        {
            let (snapshot, materialization) = TrieSnapshot::from_prehashed(root, rlp_by_digest)
                .map_err(|_| {
                    alloy_rlp::Error::Custom("failed to inspect resolved MPT cache topology")
                })?;
            let inner = CachedTrie::from_prehashed_nodes(root, rlp_by_digest)?;
            Ok(Self {
                inner,
                phantom: PhantomData,
                cached_paths: snapshot.cached_paths_on_prehashed_load(),
                materialization,
                snapshot,
                #[cfg(test)]
                snapshot_rebuilds: 1,
            })
        }
        #[cfg(not(feature = "mpt-metrics"))]
        Ok(Self::new(CachedTrie::from_prehashed_nodes(
            root,
            rlp_by_digest,
        )?))
    }

    fn get(&self, key: impl AsRef<[u8]>) -> alloy_rlp::Result<Option<T>> {
        self.inner.get(key).map(alloy_rlp::decode_exact).transpose()
    }

    #[cfg(not(feature = "mpt-metrics"))]
    fn insert(&mut self, key: impl AsRef<[u8]>, value: T) {
        self.inner.insert(key, alloy_rlp::encode(value));
    }

    #[cfg(all(feature = "mpt-metrics", test))]
    fn insert(&mut self, key: impl AsRef<[u8]>, value: T) {
        self.inner.insert(key, alloy_rlp::encode(value));
        self.rebuild_snapshot()
            .expect("test trie must remain within the snapshot bound");
    }

    #[cfg(not(feature = "mpt-metrics"))]
    fn remove(&mut self, key: impl AsRef<[u8]>) -> bool {
        self.inner.remove(key)
    }

    #[cfg(not(feature = "mpt-metrics"))]
    fn hash(&mut self) -> B256 {
        self.inner.hash()
    }

    fn size(&self) -> usize {
        self.inner.size()
    }

    fn rlp_nodes(&self) -> Vec<Bytes> {
        self.inner.rlp_nodes()
    }

    #[cfg(feature = "mpt-metrics")]
    const fn snapshot(&self) -> &TrieSnapshot {
        &self.snapshot
    }

    #[cfg(feature = "mpt-metrics")]
    fn observe_path(&self, key: impl AsRef<[u8]>) -> Result<PathObservation, MptObservationError> {
        self.snapshot.observe_path(key)
    }

    #[cfg(feature = "mpt-metrics")]
    fn rebuild_snapshot(&mut self) -> Result<(), MptObservationError> {
        TrieSnapshot::ensure_node_bound(self.inner.size())?;
        self.snapshot = TrieSnapshot::from_rlp_nodes(&self.inner.rlp_nodes())?;
        #[cfg(test)]
        {
            self.snapshot_rebuilds += 1;
        }
        Ok(())
    }

    #[cfg(feature = "mpt-metrics")]
    const fn materialization_observation(&self) -> MptMaterializationMetrics {
        self.materialization
    }

    #[cfg(feature = "mpt-metrics")]
    fn insert_observed(
        &mut self,
        key: impl AsRef<[u8]>,
        value: T,
    ) -> Result<MutationObservation, MptObservationError> {
        let key = key.as_ref();
        let path = self.snapshot.observe_path(key)?;
        let before = std::mem::take(&mut self.snapshot);
        self.inner.insert(key, alloy_rlp::encode(value));
        self.rebuild_snapshot()?;
        Ok(observe_mutation(
            &before,
            &self.snapshot,
            path,
            true,
            &mut self.cached_paths,
        ))
    }

    #[cfg(feature = "mpt-metrics")]
    fn remove_observed(
        &mut self,
        key: impl AsRef<[u8]>,
    ) -> Result<MutationObservation, MptObservationError> {
        let key = key.as_ref();
        let path = self.snapshot.observe_path(key)?;
        let before = std::mem::take(&mut self.snapshot);
        let removed = self.inner.remove(key);
        if removed {
            self.rebuild_snapshot()?;
            Ok(observe_mutation(
                &before,
                &self.snapshot,
                path,
                true,
                &mut self.cached_paths,
            ))
        } else {
            let observation =
                observe_mutation(&before, &before, path, false, &mut self.cached_paths);
            self.snapshot = before;
            Ok(observation)
        }
    }

    #[cfg(feature = "mpt-metrics")]
    fn hash_observed(&mut self) -> (B256, HashObservation) {
        let (observation, newly_cached_paths) = self.snapshot.hash_observation(&self.cached_paths);
        let root = self.inner.hash();
        self.cached_paths.extend(newly_cached_paths);
        (root, observation)
    }
}

/// Represents a sparse version of the Ethereum world state.
/// This is significantly more performant than the Reth default.
#[derive(Debug, Clone)]
#[allow(clippy::redundant_pub_crate)]
pub(super) struct SparseState {
    /// state MPT containing all used accounts
    state: RlpTrie<TrieAccount>,
    /// storage MPTs sorted by the hashed address of their account
    storages: RefCell<B256Map<RlpTrie<U256>>>,

    /// all relevant MPT nodes by their Keccak hash
    rlp_by_digest: B256Map<Bytes>,

    #[cfg(feature = "mpt-metrics")]
    metrics: RefCell<MptWorkloadMetrics>,
}

#[cfg(feature = "mpt-metrics")]
type RlpByDigestResult = (B256Map<Bytes>, MptHydrationMetrics);
#[cfg(not(feature = "mpt-metrics"))]
type RlpByDigestResult = B256Map<Bytes>;

impl SparseState {
    #[cfg(not(feature = "mpt-metrics"))]
    fn rlp_by_digest(
        witness: &ExecutionWitness,
        shared_state_nodes: &[WitnessStateNode],
    ) -> Result<RlpByDigestResult, StatelessValidationError> {
        if witness.state_indices.is_empty() {
            return Ok(witness
                .state
                .iter()
                .map(|node| (keccak256(&node.bytes), node.bytes.clone()))
                .collect());
        }

        witness
            .state_indices
            .iter()
            .map(|index| {
                let node = shared_state_nodes.get(*index as usize).ok_or(
                    StatelessValidationError::SharedWitnessStateIndexOutOfBounds {
                        index: *index,
                        len: shared_state_nodes.len(),
                    },
                )?;
                Ok((keccak256(&node.bytes), node.bytes.clone()))
            })
            .collect()
    }

    #[cfg(feature = "mpt-metrics")]
    fn rlp_by_digest(
        witness: &ExecutionWitness,
        shared_state_nodes: &[WitnessStateNode],
    ) -> Result<RlpByDigestResult, StatelessValidationError> {
        let capacity = if witness.state_indices.is_empty() {
            witness.state.len()
        } else {
            witness.state_indices.len()
        };
        let mut rlp_by_digest =
            B256Map::with_capacity_and_hasher(capacity, FbBuildHasher::<32>::default());
        let mut hydration = MptHydrationMetrics::default();

        if witness.state_indices.is_empty() {
            for node in &witness.state {
                hydration.record_rlp(node.bytes.len());
                rlp_by_digest.insert(keccak256(&node.bytes), node.bytes.clone());
            }
        } else {
            for index in &witness.state_indices {
                let node = shared_state_nodes.get(*index as usize).ok_or(
                    StatelessValidationError::SharedWitnessStateIndexOutOfBounds {
                        index: *index,
                        len: shared_state_nodes.len(),
                    },
                )?;
                hydration.record_rlp(node.bytes.len());
                rlp_by_digest.insert(keccak256(&node.bytes), node.bytes.clone());
            }
        }

        Ok((rlp_by_digest, hydration))
    }

    /// Removes an account from the state.
    #[allow(clippy::unnecessary_wraps)]
    fn remove_account(&mut self, hashed_address: &B256) -> Result<(), StatelessValidationError> {
        #[cfg(feature = "mpt-metrics")]
        {
            let observation = self
                .state
                .remove_observed(hashed_address)
                .map_err(observation_validation_error)?;
            self.metrics.get_mut().account_deletes.record(&observation);
            if let Some(storage) = self.storages.get_mut().remove(hashed_address) {
                let (nodes, bytes) = storage.snapshot().node_counts_and_bytes();
                let metrics = self.metrics.get_mut();
                metrics.materialized_storage_drop_operations += 1;
                metrics.materialized_storage_drop_nodes.add_assign(nodes);
                metrics.materialized_storage_drop_encoded_bytes += bytes;
            }
        }
        #[cfg(not(feature = "mpt-metrics"))]
        {
            self.state.remove(hashed_address);
            self.storages.get_mut().remove(hashed_address);
        }
        Ok(())
    }

    /// Clears an account's parent storage before applying a recreated account's storage delta.
    #[allow(clippy::unnecessary_wraps)]
    fn clear_storage(
        &mut self,
        hashed_address: B256,
    ) -> Result<&mut RlpTrie<U256>, StatelessValidationError> {
        #[cfg(feature = "mpt-metrics")]
        let dropped = self
            .storages
            .get_mut()
            .get(&hashed_address)
            .map(|trie| trie.snapshot().node_counts_and_bytes());

        let storage = self
            .storages
            .get_mut()
            .entry(hashed_address)
            .insert_entry(RlpTrie::default())
            .into_mut();
        #[cfg(feature = "mpt-metrics")]
        {
            let metrics = self.metrics.get_mut();
            metrics.storage_wipe_operations += 1;
            if let Some((nodes, bytes)) = dropped {
                metrics.storage_wipe_dropped_nodes.add_assign(nodes);
                metrics.storage_wipe_dropped_encoded_bytes += bytes;
            }
        }
        Ok(storage)
    }

    /// Returns a mutable version of the storage trie of the given account.
    fn storage_trie_mut(
        &mut self,
        hashed_address: B256,
    ) -> Result<&mut RlpTrie<U256>, ProviderError> {
        let trie = match self.storages.get_mut().entry(hashed_address) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                // build the storage trie matching the storage root of the account
                #[cfg(feature = "mpt-metrics")]
                {
                    let observation = self
                        .state
                        .observe_path(hashed_address)
                        .map_err(observation_provider_error)?;
                    self.metrics.get_mut().account_reads.record(&observation);
                }
                let storage_root = self
                    .state
                    .get(hashed_address)?
                    .map_or(EMPTY_ROOT_HASH, |a| a.storage_root);
                let trie = RlpTrie::from_prehashed(storage_root, &self.rlp_by_digest)
                    .map_err(ProviderError::other)?;
                #[cfg(feature = "mpt-metrics")]
                self.metrics
                    .get_mut()
                    .storage_materializations
                    .add_assign(trie.materialization_observation());
                entry.insert(trie)
            }
        };

        Ok(trie)
    }

    pub(super) fn state_node_count(&self) -> usize {
        self.state.size()
    }

    pub(super) fn storage_trie_count(&self) -> usize {
        self.storages.borrow().len()
    }

    pub(super) fn storage_node_count(&self) -> usize {
        self.storages.borrow().values().map(RlpTrie::size).sum()
    }

    pub(super) fn materialized_witness_state_nodes(&self) -> Vec<WitnessStateNode> {
        let mut nodes = self
            .state
            .rlp_nodes()
            .into_iter()
            .map(WitnessStateNode::from_bytes)
            .collect::<Vec<_>>();
        nodes.extend(
            self.storages
                .borrow()
                .values()
                .flat_map(RlpTrie::rlp_nodes)
                .map(WitnessStateNode::from_bytes),
        );
        ExecutionWitness::canonicalize_state_nodes(nodes)
    }

    #[cfg(feature = "mpt-metrics")]
    pub(super) fn mpt_workload_metrics(&self) -> MptWorkloadMetrics {
        *self.metrics.borrow()
    }

    pub(super) fn storage_from_witness(
        &mut self,
        address: Address,
        slot: U256,
    ) -> Result<U256, ProviderError> {
        self.account(address)?;
        let hashed_address = keccak256(address);
        let hashed_slot = keccak256(B256::from(slot));
        let storage_trie = self.storage_trie_mut(hashed_address).map_err(|err| {
            ProviderError::TrieWitnessError(format!("storage trie missing for {address}: {err}"))
        })?;
        #[cfg(feature = "mpt-metrics")]
        let observation = storage_trie
            .observe_path(hashed_slot)
            .map_err(observation_provider_error)?;
        let value = catch_unwind(AssertUnwindSafe(|| storage_trie.get(hashed_slot))).map_err(
            |_| {
                ProviderError::TrieWitnessError(format!(
                    "storage trie unresolved while address {address} slot {slot} (hashed {hashed_slot})"
                ))
            },
        )??;
        #[cfg(feature = "mpt-metrics")]
        self.metrics.get_mut().storage_reads.record(&observation);
        Ok(value.unwrap_or(U256::ZERO))
    }
}

#[cfg(feature = "mpt-metrics")]
#[allow(clippy::needless_pass_by_value)]
fn observation_provider_error(error: MptObservationError) -> ProviderError {
    ProviderError::TrieWitnessError(format!("MPT metrics observation failed: {error}"))
}

#[cfg(feature = "mpt-metrics")]
#[allow(clippy::needless_pass_by_value)]
fn observation_validation_error(error: MptObservationError) -> StatelessValidationError {
    StatelessValidationError::StatelessExecutionFailed(format!(
        "MPT metrics observation failed: {error}"
    ))
}

impl StatelessTrie for SparseState {
    /// Initialize the stateless trie using the `ExecutionWitness`.
    fn new(
        witness: &ExecutionWitness,
        pre_state_root: B256,
    ) -> Result<(Self, B256Map<Bytecode>), StatelessValidationError> {
        Self::new_with_state_pool(witness, &[], pre_state_root)
    }

    fn new_with_state_pool(
        witness: &ExecutionWitness,
        shared_state_nodes: &[WitnessStateNode],
        pre_state_root: B256,
    ) -> Result<(Self, B256Map<Bytecode>), StatelessValidationError> {
        // First, hash all the RLP nodes once.
        #[cfg(feature = "mpt-metrics")]
        let (rlp_by_digest, witness_hydration) = Self::rlp_by_digest(witness, shared_state_nodes)?;
        #[cfg(not(feature = "mpt-metrics"))]
        let rlp_by_digest = Self::rlp_by_digest(witness, shared_state_nodes)?;

        // construct the state trie from the witness data and the given state root
        let state = RlpTrie::from_prehashed(pre_state_root, &rlp_by_digest)
            .map_err(|_| StatelessValidationError::WitnessRevealFailed { pre_state_root })?;
        #[cfg(feature = "mpt-metrics")]
        let state_materialization = state.materialization_observation();

        // hash all the supplied bytecode
        let bytecode = witness
            .codes
            .iter()
            .map(|code| (keccak256(code), Bytecode::new_raw(code.clone())))
            .collect();

        Ok((
            Self {
                state,
                storages: RefCell::new(B256Map::default()),
                rlp_by_digest,
                #[cfg(feature = "mpt-metrics")]
                metrics: RefCell::new(MptWorkloadMetrics {
                    witness_hydration,
                    state_materializations: state_materialization,
                    ..Default::default()
                }),
            },
            bytecode,
        ))
    }

    /// Returns the `TrieAccount` that corresponds to the `Address`.
    fn account(&self, address: Address) -> Result<Option<TrieAccount>, ProviderError> {
        let hashed_address = keccak256(address);
        #[cfg(feature = "mpt-metrics")]
        let observation = self
            .state
            .observe_path(hashed_address)
            .map_err(observation_provider_error)?;
        let account = catch_unwind(AssertUnwindSafe(|| self.state.get(hashed_address))).map_err(
            |_| {
                ProviderError::TrieWitnessError(format!(
                    "state trie unresolved while loading account {address} (hashed {hashed_address})"
                ))
            },
        )??;
        #[cfg(feature = "mpt-metrics")]
        self.metrics.borrow_mut().account_reads.record(&observation);

        match account {
            None => Ok(None),
            Some(account) => {
                // each time an account is accessed, check whether its storage trie already exists
                // otherwise construct it from the witness data and the account's storage root
                match self.storages.borrow_mut().entry(hashed_address) {
                    Entry::Vacant(entry) => {
                        let trie =
                            RlpTrie::from_prehashed(account.storage_root, &self.rlp_by_digest)?;
                        #[cfg(feature = "mpt-metrics")]
                        {
                            self.metrics
                                .borrow_mut()
                                .storage_materializations
                                .add_assign(trie.materialization_observation());
                        }
                        entry.insert(trie);
                    }
                    Entry::Occupied(_) => {}
                }

                Ok(Some(account))
            }
        }
    }

    /// Returns the storage slot value that corresponds to the given (address, slot) tuple.
    fn storage(&self, address: Address, slot: U256) -> Result<U256, ProviderError> {
        let storages = self.storages.borrow();
        let storage_trie = storages.get(&keccak256(address)).ok_or_else(|| {
            ProviderError::TrieWitnessError(format!("storage trie missing for {address}"))
        })?;
        let hashed_slot = keccak256(B256::from(slot));
        #[cfg(feature = "mpt-metrics")]
        let observation = storage_trie
            .observe_path(hashed_slot)
            .map_err(observation_provider_error)?;
        let value = catch_unwind(AssertUnwindSafe(|| storage_trie.get(hashed_slot))).map_err(
            |_| {
                ProviderError::TrieWitnessError(format!(
                    "storage trie unresolved while loading address {address} slot {slot} (hashed {hashed_slot})"
                ))
            },
        )??;
        #[cfg(feature = "mpt-metrics")]
        self.metrics.borrow_mut().storage_reads.record(&observation);
        Ok(value.unwrap_or(U256::ZERO))
    }

    /// Computes the new state root from the `HashedPostState`.
    #[allow(clippy::too_many_lines)]
    fn calculate_state_root(
        &mut self,
        state: HashedPostState,
        pre_existing_destroyed_accounts: &[B256],
    ) -> Result<B256, StatelessValidationError> {
        // The mutation sequence is part of the measured workload: HashMap iteration order is not
        // portable between the native host and zkVM. Canonical sorting keeps split/merge paths and
        // their hashing work deterministic on both sides.
        let state = state.into_sorted();
        let mut removed_accounts = Vec::new();
        for (hashed_address, account) in state.accounts {
            // nonexisting accounts must be removed from the state
            let Some(account) = account else {
                removed_accounts.push(hashed_address);
                continue;
            };

            // Apply storage changes before computing the storage root.
            #[cfg(feature = "mpt-metrics")]
            let storage_root = {
                let mut inserts_and_updates = Vec::new();
                let mut deletes = Vec::new();
                let (root, hash_observation) = {
                    // reth v2.6 post-state storage contains only explicitly changed slots. Its
                    // separate destroyed-account metadata tells us when the delta starts from an
                    // empty trie; otherwise unmentioned parent slots must be retained.
                    let storage_trie = if pre_existing_destroyed_accounts
                        .binary_search(&hashed_address)
                        .is_ok()
                    {
                        self.clear_storage(hashed_address)?
                    } else {
                        self.storage_trie_mut(hashed_address).map_err(|error| {
                            StatelessValidationError::StatelessExecutionFailed(error.to_string())
                        })?
                    };

                    if let Some(storage) = state.storages.get(&hashed_address) {
                        for (hashed_key, value) in &storage.storage_slots {
                            if !value.is_zero() {
                                let observation = storage_trie
                                    .insert_observed(hashed_key, *value)
                                    .map_err(observation_validation_error)?;
                                inserts_and_updates.push(observation);
                            }
                        }
                        // Removals happen last so unresolved orphans cannot be exposed early.
                        for (hashed_key, value) in &storage.storage_slots {
                            if value.is_zero() {
                                deletes.push(
                                    storage_trie
                                        .remove_observed(hashed_key)
                                        .map_err(observation_validation_error)?,
                                );
                            }
                        }
                    }
                    storage_trie.hash_observed()
                };

                let metrics = self.metrics.get_mut();
                for observation in inserts_and_updates {
                    if observation.path.found {
                        metrics.storage_updates.record(&observation);
                    } else {
                        metrics.storage_inserts.record(&observation);
                    }
                }
                for observation in deletes {
                    metrics.storage_deletes.record(&observation);
                }
                metrics.storage_hashes.record(hash_observation);
                root
            };

            #[cfg(not(feature = "mpt-metrics"))]
            let storage_root = {
                let storage_trie = if pre_existing_destroyed_accounts
                    .binary_search(&hashed_address)
                    .is_ok()
                {
                    self.clear_storage(hashed_address)?
                } else {
                    self.storage_trie_mut(hashed_address).map_err(|error| {
                        StatelessValidationError::StatelessExecutionFailed(error.to_string())
                    })?
                };
                if let Some(storage) = state.storages.get(&hashed_address) {
                    for (hashed_key, value) in &storage.storage_slots {
                        if !value.is_zero() {
                            storage_trie.insert(hashed_key, *value);
                        }
                    }
                    for (hashed_key, value) in &storage.storage_slots {
                        if value.is_zero() {
                            storage_trie.remove(hashed_key);
                        }
                    }
                }
                storage_trie.hash()
            };

            // update/insert the account after all changes have been processed
            let account = TrieAccount {
                nonce: account.nonce,
                balance: account.balance,
                storage_root,
                code_hash: account.bytecode_hash.unwrap_or(KECCAK256_EMPTY),
            };
            #[cfg(feature = "mpt-metrics")]
            {
                let observation = self
                    .state
                    .insert_observed(hashed_address, account)
                    .map_err(observation_validation_error)?;
                if observation.path.found {
                    self.metrics.get_mut().account_updates.record(&observation);
                } else {
                    self.metrics.get_mut().account_inserts.record(&observation);
                }
            }
            #[cfg(not(feature = "mpt-metrics"))]
            self.state.insert(hashed_address, account);
        }
        for hashed_address in &removed_accounts {
            self.remove_account(hashed_address)?;
        }

        #[cfg(feature = "mpt-metrics")]
        {
            let (root, observation) = self.state.hash_observed();
            self.metrics.get_mut().account_hashes.record(observation);
            Ok(root)
        }
        #[cfg(not(feature = "mpt-metrics"))]
        {
            Ok(self.state.hash())
        }
    }
}

#[cfg(all(test, feature = "mpt-metrics"))]
mod mpt_metrics_tests {
    use super::{RlpTrie, SparseState};
    use crate::trie::StatelessTrie;
    use alloy_primitives::{Address, B256, Bytes, U256, keccak256, map::B256Map};
    use alloy_trie::EMPTY_ROOT_HASH;
    use raiko2_primitives::{ExecutionWitness, WitnessStateNode};
    use reth_primitives_traits::Account;
    use reth_trie_common::{HashedPostState, HashedStorage};

    use risc0_ethereum_trie::CachedTrie;

    #[test]
    fn path_observer_counts_empty_hit_and_leaf_miss() {
        let mut trie = RlpTrie::<U256>::default();
        let key = B256::repeat_byte(0x11);

        let empty = trie.observe_path(key).unwrap();
        assert_eq!(empty.node_visits, 1);
        assert!(!empty.found);

        trie.insert(key, U256::from(7));
        let hit = trie.observe_path(key).unwrap();
        assert_eq!(hit.node_visits, 1);
        assert!(hit.found);

        let miss = trie.observe_path(B256::repeat_byte(0x12)).unwrap();
        assert_eq!(miss.node_visits, 1);
        assert!(!miss.found);
    }

    #[test]
    fn path_observer_stops_at_branch_when_child_is_absent() {
        let mut trie = RlpTrie::<U256>::default();
        trie.insert(B256::repeat_byte(0x10), U256::from(1));
        trie.insert(B256::repeat_byte(0x20), U256::from(2));

        let miss = trie.observe_path(B256::repeat_byte(0x30)).unwrap();
        assert_eq!(miss.node_visits, 1);
        assert_eq!(miss.visited_nodes.branch, 1);
        assert_eq!(miss.visited_nodes.null, 0);
        assert!(!miss.found);
    }

    #[test]
    fn path_observer_resolves_inline_and_hashed_children() {
        let mut inline = RlpTrie::<U256>::default();
        inline.insert([0x10], U256::from(1));
        inline.insert([0x20], U256::from(2));
        assert_eq!(inline.inner.rlp_nodes().len(), 1);
        assert_eq!(inline.observe_path([0x10]).unwrap().node_visits, 2);

        let mut hashed = RlpTrie::<U256>::default();
        let first = B256::repeat_byte(0x10);
        hashed.insert(first, U256::from(1));
        hashed.insert(B256::repeat_byte(0x20), U256::from(2));
        assert!(hashed.inner.rlp_nodes().len() > 1);
        assert_eq!(hashed.observe_path(first).unwrap().node_visits, 2);
    }

    #[test]
    fn repeated_reads_reuse_the_persistent_snapshot() {
        let mut trie = RlpTrie::<U256>::default();
        let key = B256::repeat_byte(0x10);
        trie.insert(key, U256::from(1));
        let rebuilds = trie.snapshot_rebuilds;

        for _ in 0..100 {
            assert!(trie.observe_path(key).unwrap().found);
        }

        assert_eq!(trie.snapshot_rebuilds, rebuilds);
    }

    #[test]
    fn insert_split_changes_later_current_path_depth() {
        let mut trie = RlpTrie::<U256>::default();
        let first = B256::repeat_byte(0x10);
        let second = B256::repeat_byte(0x20);
        trie.insert(first, U256::from(1));

        let mutation = trie.insert_observed(second, U256::from(2)).unwrap();
        assert_eq!(mutation.path.node_visits, 1);
        assert!(!mutation.path.found);
        assert_eq!(mutation.delta.created_nodes.branch, 1);
        assert_eq!(mutation.delta.created_nodes.leaf, 2);
        assert_eq!(mutation.delta.removed_nodes.leaf, 1);
        assert_eq!(trie.observe_path(first).unwrap().node_visits, 2);
    }

    #[test]
    fn delete_collapse_changes_later_current_path_depth() {
        let mut trie = RlpTrie::<U256>::default();
        let first = B256::repeat_byte(0x10);
        let second = B256::repeat_byte(0x20);
        trie.insert(first, U256::from(1));
        trie.insert(second, U256::from(2));

        let mutation = trie.remove_observed(second).unwrap();
        assert_eq!(mutation.path.node_visits, 2);
        assert!(mutation.path.found);
        assert!(mutation.succeeded);
        assert_eq!(mutation.delta.removed_nodes.branch, 1);
        assert_eq!(mutation.delta.removed_nodes.leaf, 2);
        assert_eq!(mutation.delta.created_nodes.leaf, 1);
        assert_eq!(trie.observe_path(first).unwrap().node_visits, 1);
    }

    #[test]
    fn hash_observation_deduplicates_dirty_ancestors_across_mutations() {
        let mut trie = RlpTrie::<U256>::default();
        let first = B256::repeat_byte(0x10);
        let second = B256::repeat_byte(0x20);
        trie.insert_observed(first, U256::from(1)).unwrap();
        trie.insert_observed(second, U256::from(2)).unwrap();

        let (_, observation) = trie.hash_observed();
        assert_eq!(observation.dirty_encoded_nodes, 3);
        assert_eq!(observation.dirty_hashed_nodes, 3);
        assert!(observation.hash_input_bytes > 0);
        assert!(observation.hash_input_blocks >= observation.dirty_hashed_nodes);

        let (_, clean) = trie.hash_observed();
        assert_eq!(clean.dirty_encoded_nodes, 0);
        assert_eq!(clean.dirty_hashed_nodes, 0);
    }

    #[test]
    fn same_value_update_still_records_cache_invalidation_hash_work() {
        let mut trie = RlpTrie::<U256>::default();
        let key = B256::repeat_byte(0x10);
        trie.insert(key, U256::from(1));

        let update = trie.insert_observed(key, U256::from(1)).unwrap();
        assert!(update.path.found);
        assert_eq!(update.delta.dirty_encoded_nodes, 0);

        let (_, hash) = trie.hash_observed();
        assert_eq!(hash.dirty_encoded_nodes, 1);
        assert_eq!(hash.dirty_hashed_nodes, 1);
        assert!(hash.hash_input_bytes > 0);
    }

    #[test]
    fn hydrated_inline_sibling_is_encoded_after_parent_invalidation() {
        let mut source = RlpTrie::<U256>::default();
        source.insert([0x10], U256::from(1));
        source.insert([0x20], U256::from(2));
        let root = source.inner.hash();
        let rlp_by_digest = source
            .inner
            .rlp_nodes()
            .into_iter()
            .map(|rlp| (keccak256(&rlp), rlp))
            .collect::<B256Map<_>>();
        let mut trie = RlpTrie::<U256>::from_prehashed(root, &rlp_by_digest).unwrap();

        trie.insert_observed([0x10], U256::from(3)).unwrap();
        let (_, hash) = trie.hash_observed();

        // The branch and both inline leaves are encoded. The untouched sibling starts with an
        // empty cache and is therefore memoized when the invalidated branch is hashed.
        assert_eq!(hash.dirty_encoded_nodes, 3);
        assert_eq!(hash.dirty_hashed_nodes, 1);
    }

    #[test]
    fn raw_inserted_trie_encodes_all_nodes_at_first_hash_boundary() {
        let mut trie = RlpTrie::<U256>::default();
        trie.insert([0x10], U256::from(1));
        trie.insert([0x20], U256::from(2));

        let (_, hash) = trie.hash_observed();

        assert_eq!(hash.dirty_encoded_nodes, 3);
        assert_eq!(hash.dirty_hashed_nodes, 1);
    }

    #[test]
    fn untouched_cached_digest_barrier_hides_its_uncached_inline_descendants() {
        let value = U256::from(1) << 80;
        let mut source = RlpTrie::<U256>::default();
        for key in [[0x10], [0x11], [0x20], [0x21]] {
            source.insert(key, value);
        }
        let root = source.inner.hash();
        let rlp_by_digest = source
            .inner
            .rlp_nodes()
            .into_iter()
            .map(|rlp| (keccak256(&rlp), rlp))
            .collect::<B256Map<_>>();
        let mut trie = RlpTrie::<U256>::from_prehashed(root, &rlp_by_digest).unwrap();

        trie.insert_observed([0x10], value + U256::from(1)).unwrap();
        let (_, hash) = trie.hash_observed();

        // Root, the changed hashed child branch, and its two inline leaves are memoized. The
        // other hashed child remains a cache barrier, so neither it nor its inline leaves run.
        assert_eq!(hash.dirty_encoded_nodes, 4);
        assert_eq!(hash.dirty_hashed_nodes, 2);
    }

    #[test]
    fn prehashed_materialization_counts_hashed_and_inline_decode_work() {
        let mut source = RlpTrie::<U256>::default();
        source.insert([0x10], U256::from(1));
        source.insert([0x20], U256::from(2));
        let root = source.inner.hash();
        let nodes = source.inner.rlp_nodes();
        assert_eq!(nodes.len(), 1);
        let root_rlp_len = nodes[0].len() as u64;
        let rlp_by_digest = nodes
            .into_iter()
            .map(|rlp| (keccak256(&rlp), rlp))
            .collect::<B256Map<_>>();

        let trie = RlpTrie::<U256>::from_prehashed(root, &rlp_by_digest).unwrap();
        let materialization = trie.materialization_observation();

        assert_eq!(materialization.resolved_hashed_rlp_nodes, 1);
        assert_eq!(materialization.resolved_hashed_rlp_bytes, root_rlp_len);
        assert_eq!(materialization.decoded_nodes.branch, 1);
        assert_eq!(materialization.decoded_nodes.leaf, 2);
        assert_eq!(materialization.resolved_digest_lookups, 1);
        assert_eq!(materialization.unresolved_digest_lookups, 0);
    }

    #[test]
    fn prehashed_materialization_counts_unresolved_digest_edges() {
        let mut source = RlpTrie::<U256>::default();
        source.insert(B256::repeat_byte(0x10), U256::from(1));
        source.insert(B256::repeat_byte(0x20), U256::from(2));
        let root = source.inner.hash();
        let nodes = source.inner.rlp_nodes();
        assert_eq!(nodes.len(), 3);
        let missing_digest = keccak256(nodes.last().unwrap());
        let mut rlp_by_digest = nodes
            .into_iter()
            .map(|rlp| (keccak256(&rlp), rlp))
            .collect::<B256Map<_>>();
        rlp_by_digest.remove(&missing_digest);

        let trie = RlpTrie::<U256>::from_prehashed(root, &rlp_by_digest).unwrap();
        let materialization = trie.materialization_observation();

        assert_eq!(materialization.resolved_hashed_rlp_nodes, 2);
        assert_eq!(materialization.decoded_nodes.branch, 1);
        assert_eq!(materialization.decoded_nodes.leaf, 1);
        assert_eq!(materialization.resolved_digest_lookups, 2);
        assert_eq!(materialization.unresolved_digest_lookups, 1);
    }

    #[test]
    fn dirty_subtree_moved_by_later_split_is_charged_at_its_final_path() {
        let mut trie = RlpTrie::<U256>::default();
        let first = B256::with_last_byte(0x10);
        let second = B256::with_last_byte(0x11);
        let outside_prefix = B256::repeat_byte(0x20);
        trie.insert(first, U256::from(1));
        trie.insert(second, U256::from(2));
        trie.hash_observed();

        // The update invalidates caches inside the existing subtree. The following split moves
        // that still-dirty subtree to another structural path before the next hash boundary.
        trie.insert_observed(first, U256::from(1)).unwrap();
        let split = trie.insert_observed(outside_prefix, U256::from(3)).unwrap();
        assert!(split.delta.created_nodes.branch >= 1);
        assert!(split.delta.created_nodes.extension >= 1);

        let (_, hash) = trie.hash_observed();
        assert_eq!(hash.dirty_encoded_nodes, 5);
        assert_eq!(hash.dirty_hashed_nodes, 3);
    }

    #[test]
    fn extension_split_and_merge_are_present_in_structural_deltas() {
        let mut trie = RlpTrie::<U256>::default();
        let first = B256::with_last_byte(0x10);
        let second = B256::with_last_byte(0x11);
        let outside_prefix = B256::repeat_byte(0x20);
        trie.insert(first, U256::from(1));
        trie.insert(second, U256::from(2));

        let split = trie.insert_observed(outside_prefix, U256::from(3)).unwrap();
        assert!(split.delta.removed_nodes.extension >= 1);
        assert!(split.delta.created_nodes.branch >= 1);

        let merge = trie.remove_observed(outside_prefix).unwrap();
        assert!(merge.delta.removed_nodes.branch >= 1);
        assert!(merge.delta.created_nodes.extension >= 1);
    }

    #[test]
    fn unresolved_target_hash_fails_closed() {
        let mut full = RlpTrie::<U256>::default();
        let target = B256::repeat_byte(0x10);
        full.insert(target, U256::from(1));
        full.insert(B256::repeat_byte(0x20), U256::from(2));
        let root_only = full
            .inner
            .rlp_nodes()
            .into_iter()
            .take(1)
            .collect::<Vec<_>>();
        let partial = RlpTrie::<U256>::new(CachedTrie::from_rlp(root_only).unwrap());

        let error = partial.observe_path(target).unwrap_err();
        assert!(error.to_string().contains("unresolved branch child"));
    }

    #[test]
    fn bounded_snapshot_handles_moderate_materialized_trie() {
        let mut trie = RlpTrie::<U256>::default();
        for index in 0..256u64 {
            trie.insert(B256::from(U256::from(index)), U256::from(index + 1));
        }

        let snapshot = trie.snapshot();
        let (nodes, encoded_bytes) = snapshot.node_counts_and_bytes();
        assert_eq!(nodes.leaf, 256);
        assert!(nodes.branch > 0);
        assert!(encoded_bytes > 256);

        let update = trie
            .insert_observed(B256::from(U256::from(128)), U256::from(999))
            .unwrap();
        assert!(update.path.found);
        assert!(update.path.node_visits > 1);
    }

    #[test]
    fn sparse_state_keeps_account_and_storage_read_metrics_separate() {
        let (state, _) =
            <SparseState as StatelessTrie>::new(&ExecutionWitness::default(), EMPTY_ROOT_HASH)
                .unwrap();

        assert_eq!(state.account(Address::ZERO).unwrap(), None);
        let metrics = state.mpt_workload_metrics();
        assert_eq!(metrics.state_materializations.operations, 1);
        assert_eq!(metrics.storage_materializations.operations, 0);
        assert_eq!(metrics.account_reads.operations, 1);
        assert_eq!(metrics.account_reads.visited_nodes.null, 1);
        assert_eq!(metrics.storage_reads.operations, 0);
        let serialized = serde_json::to_value(metrics).unwrap();
        assert_eq!(serialized["account_reads"]["operations"], 1);
    }

    fn account(nonce: u64) -> Account {
        Account {
            nonce,
            balance: U256::from(nonce * 10),
            bytecode_hash: None,
        }
    }

    fn seeded_state() -> SparseState {
        let (mut trie, _) =
            <SparseState as StatelessTrie>::new(&ExecutionWitness::default(), EMPTY_ROOT_HASH)
                .unwrap();
        let addresses = [
            B256::repeat_byte(0x10),
            B256::repeat_byte(0x20),
            B256::repeat_byte(0x30),
        ];
        let seed = HashedPostState::default()
            .with_accounts([
                (addresses[0], Some(account(1))),
                (addresses[1], Some(account(2))),
                (addresses[2], Some(account(3))),
            ])
            .with_storages([(
                addresses[0],
                HashedStorage::from_iter([
                    (B256::repeat_byte(0x11), U256::from(11)),
                    (B256::repeat_byte(0x22), U256::from(22)),
                    (B256::repeat_byte(0x33), U256::from(33)),
                ]),
            )]);
        trie.calculate_state_root(seed, &[]).unwrap();
        trie
    }

    fn update_state(reverse: bool) -> HashedPostState {
        let mut accounts = vec![
            (B256::repeat_byte(0x10), Some(account(1))),
            (B256::repeat_byte(0x20), None),
            (B256::repeat_byte(0x30), Some(account(4))),
        ];
        let mut slots = vec![
            (B256::repeat_byte(0x11), U256::from(111)),
            (B256::repeat_byte(0x22), U256::ZERO),
            (B256::repeat_byte(0x44), U256::from(44)),
        ];
        if reverse {
            accounts.reverse();
            slots.reverse();
        }
        HashedPostState::default()
            .with_accounts(accounts)
            .with_storages([(B256::repeat_byte(0x10), HashedStorage::from_iter(slots))])
    }

    #[test]
    fn state_and_storage_materialization_metrics_are_separate() {
        let state = seeded_state();
        let metrics = state.mpt_workload_metrics();

        assert_eq!(metrics.state_materializations.operations, 1);
        assert_eq!(metrics.storage_materializations.operations, 3);
    }

    #[test]
    fn canonical_account_storage_and_delete_order_has_stable_root_and_metrics() {
        let mut forward = seeded_state();
        let mut reverse = seeded_state();
        let before = forward.mpt_workload_metrics();

        let forward_root = forward
            .calculate_state_root(update_state(false), &[])
            .unwrap();
        let reverse_root = reverse
            .calculate_state_root(update_state(true), &[])
            .unwrap();
        assert_eq!(forward_root, reverse_root);

        let (mut expected, _) =
            <SparseState as StatelessTrie>::new(&ExecutionWitness::default(), EMPTY_ROOT_HASH)
                .unwrap();
        let expected_root = expected
            .calculate_state_root(
                HashedPostState::default()
                    .with_accounts([
                        (B256::repeat_byte(0x10), Some(account(1))),
                        (B256::repeat_byte(0x30), Some(account(4))),
                    ])
                    .with_storages([(
                        B256::repeat_byte(0x10),
                        HashedStorage::from_iter([
                            (B256::repeat_byte(0x11), U256::from(111)),
                            (B256::repeat_byte(0x33), U256::from(33)),
                            (B256::repeat_byte(0x44), U256::from(44)),
                        ]),
                    )]),
                &[],
            )
            .unwrap();
        assert_eq!(forward_root, expected_root);
        assert_eq!(
            forward.mpt_workload_metrics(),
            reverse.mpt_workload_metrics()
        );

        let metrics = forward.mpt_workload_metrics();
        assert_eq!(
            metrics.storage_inserts.operations - before.storage_inserts.operations,
            1
        );
        assert_eq!(
            metrics.storage_updates.operations - before.storage_updates.operations,
            1
        );
        assert_eq!(
            metrics.storage_deletes.operations - before.storage_deletes.operations,
            1
        );
        assert_eq!(
            metrics.account_deletes.operations - before.account_deletes.operations,
            1
        );
    }

    #[test]
    fn hydration_counts_every_direct_node_before_map_deduplication() {
        let node = WitnessStateNode::from_bytes(Bytes::from_static(&[0x80]));
        let witness = ExecutionWitness {
            state: vec![node.clone(), node],
            ..Default::default()
        };

        let (by_digest, metrics) = SparseState::rlp_by_digest(&witness, &[]).unwrap();
        assert_eq!(by_digest.len(), 1);
        assert_eq!(metrics.hashed_rlp_nodes, 2);
        assert_eq!(metrics.hash_input_bytes, 2);
        assert_eq!(metrics.hash_input_blocks, 2);
    }

    #[test]
    fn hydration_counts_every_indexed_node_and_keccak_padding_block() {
        let shared = vec![WitnessStateNode::from_bytes(Bytes::from(vec![0x44; 136]))];
        let witness = ExecutionWitness {
            state_indices: vec![0, 0],
            ..Default::default()
        };

        let (by_digest, metrics) = SparseState::rlp_by_digest(&witness, &shared).unwrap();
        assert_eq!(by_digest.len(), 1);
        assert_eq!(metrics.hashed_rlp_nodes, 2);
        assert_eq!(metrics.hash_input_bytes, 272);
        assert_eq!(metrics.hash_input_blocks, 4);
    }
}

#[cfg(test)]
mod canonical_mutation_order_tests {
    use super::{RlpTrie, SparseState};
    use crate::trie::StatelessTrie;
    use alloy_consensus::TrieAccount;
    use alloy_primitives::{B256, KECCAK256_EMPTY, U256};
    use alloy_trie::EMPTY_ROOT_HASH;
    use raiko2_primitives::ExecutionWitness;
    use reth_primitives_traits::Account;
    use reth_trie_common::HashedPostState;

    fn accounts() -> Vec<(B256, Option<Account>)> {
        vec![
            (
                B256::repeat_byte(0x30),
                Some(Account {
                    nonce: 3,
                    balance: U256::from(30),
                    bytecode_hash: None,
                }),
            ),
            (
                B256::repeat_byte(0x10),
                Some(Account {
                    nonce: 1,
                    balance: U256::from(10),
                    bytecode_hash: None,
                }),
            ),
            (
                B256::repeat_byte(0x20),
                Some(Account {
                    nonce: 2,
                    balance: U256::from(20),
                    bytecode_hash: None,
                }),
            ),
        ]
    }

    fn calculate_root(entries: Vec<(B256, Option<Account>)>) -> B256 {
        let (mut trie, _) =
            <SparseState as StatelessTrie>::new(&ExecutionWitness::default(), EMPTY_ROOT_HASH)
                .unwrap();
        trie.calculate_state_root(HashedPostState::default().with_accounts(entries), &[])
            .unwrap()
    }

    #[test]
    fn canonical_post_state_order_is_root_equivalent_and_input_order_stable() {
        let entries = accounts();
        let mut reversed = entries.clone();
        reversed.reverse();
        let canonical_root = calculate_root(entries.clone());
        assert_eq!(calculate_root(reversed), canonical_root);

        let mut reference = RlpTrie::<TrieAccount>::default();
        for (hashed_address, account) in entries {
            let account = account.unwrap();
            reference.insert(
                hashed_address,
                TrieAccount {
                    nonce: account.nonce,
                    balance: account.balance,
                    storage_root: EMPTY_ROOT_HASH,
                    code_hash: KECCAK256_EMPTY,
                },
            );
        }
        assert_eq!(reference.inner.hash(), canonical_root);
    }
}

#[cfg(test)]
mod storage_delta_reth26_tests {
    use super::{RlpTrie, SparseState};
    use crate::trie::StatelessTrie;
    use alloy_consensus::TrieAccount;
    use alloy_primitives::{Address, B256, KECCAK256_EMPTY, U256, keccak256};
    use raiko2_primitives::{ExecutionWitness, WitnessStateNode};
    use reth_primitives_traits::Account;
    use reth_trie_common::{HashedPostState, HashedStorage};

    fn parent_with_storage(address: Address) -> (SparseState, B256, U256, U256, B256, B256) {
        let hashed_address = keccak256(address);
        let rewritten_slot = U256::from(1);
        let stale_parent_slot = U256::from(2);
        let hashed_rewritten_slot = keccak256(B256::from(rewritten_slot));
        let hashed_stale_parent_slot = keccak256(B256::from(stale_parent_slot));

        let mut parent_storage = RlpTrie::<U256>::default();
        parent_storage.insert(hashed_rewritten_slot, U256::from(10));
        parent_storage.insert(hashed_stale_parent_slot, U256::from(20));
        let mut parent_state = RlpTrie::<TrieAccount>::default();
        parent_state.insert(
            hashed_address,
            TrieAccount {
                nonce: 1,
                balance: U256::from(100),
                storage_root: parent_storage.inner.hash(),
                code_hash: KECCAK256_EMPTY,
            },
        );
        let pre_state_root = parent_state.inner.hash();
        let witness = ExecutionWitness {
            state: parent_state
                .rlp_nodes()
                .into_iter()
                .chain(parent_storage.rlp_nodes())
                .map(WitnessStateNode::from_bytes)
                .collect(),
            ..Default::default()
        };
        let (sparse, _) = SparseState::new(&witness, pre_state_root).unwrap();
        (
            sparse,
            hashed_address,
            rewritten_slot,
            stale_parent_slot,
            hashed_rewritten_slot,
            hashed_stale_parent_slot,
        )
    }

    fn expected_recreated_root(
        hashed_address: B256,
        account: &Account,
        storage_slots: impl IntoIterator<Item = (B256, U256)>,
    ) -> B256 {
        let mut storage = RlpTrie::<U256>::default();
        for (slot, value) in storage_slots {
            storage.insert(slot, value);
        }
        let mut state = RlpTrie::<TrieAccount>::default();
        state.insert(
            hashed_address,
            TrieAccount {
                nonce: account.nonce,
                balance: account.balance,
                storage_root: storage.inner.hash(),
                code_hash: account.bytecode_hash.unwrap_or(KECCAK256_EMPTY),
            },
        );
        state.inner.hash()
    }

    #[test]
    fn post_state_storage_delta_preserves_unmentioned_parent_slot() {
        let address = Address::repeat_byte(0x11);
        let hashed_address = keccak256(address);
        let changed_slot = U256::from(1);
        let unchanged_slot = U256::from(2);
        let hashed_changed_slot = keccak256(B256::from(changed_slot));
        let hashed_unchanged_slot = keccak256(B256::from(unchanged_slot));

        let mut storage = RlpTrie::<U256>::default();
        storage.insert(hashed_changed_slot, U256::from(10));
        storage.insert(hashed_unchanged_slot, U256::from(20));
        let storage_root = storage.inner.hash();

        let mut state = RlpTrie::<TrieAccount>::default();
        state.insert(
            hashed_address,
            TrieAccount {
                nonce: 1,
                balance: U256::from(100),
                storage_root,
                code_hash: KECCAK256_EMPTY,
            },
        );
        let pre_state_root = state.inner.hash();

        let witness = ExecutionWitness {
            state: state
                .rlp_nodes()
                .into_iter()
                .chain(storage.rlp_nodes())
                .map(WitnessStateNode::from_bytes)
                .collect(),
            ..Default::default()
        };
        let (mut sparse, _) = SparseState::new(&witness, pre_state_root).unwrap();
        let post_state = HashedPostState::default()
            .with_accounts([(
                hashed_address,
                Some(Account {
                    nonce: 1,
                    balance: U256::from(100),
                    bytecode_hash: None,
                }),
            )])
            .with_storages([(
                hashed_address,
                HashedStorage::from_iter([(hashed_changed_slot, U256::from(30))]),
            )]);

        sparse.calculate_state_root(post_state, &[]).unwrap();

        assert_eq!(
            sparse.storage(address, changed_slot).unwrap(),
            U256::from(30)
        );
        assert_eq!(
            sparse.storage(address, unchanged_slot).unwrap(),
            U256::from(20)
        );
    }

    #[test]
    fn pre_unzen_destroy_then_fund_clears_parent_storage() {
        let address = Address::repeat_byte(0x22);
        let (
            mut sparse,
            hashed_address,
            rewritten_slot,
            stale_parent_slot,
            _hashed_rewritten_slot,
            _hashed_stale_parent_slot,
        ) = parent_with_storage(address);
        let funded = Account {
            nonce: 0,
            balance: U256::from(7),
            bytecode_hash: None,
        };
        let recreated = HashedPostState::default().with_accounts([(hashed_address, Some(funded))]);

        let actual_root = sparse
            .calculate_state_root(recreated, &[hashed_address])
            .unwrap();
        assert_eq!(
            actual_root,
            expected_recreated_root(hashed_address, &funded, [])
        );
        assert_eq!(sparse.storage(address, rewritten_slot).unwrap(), U256::ZERO);
        assert_eq!(
            sparse.storage(address, stale_parent_slot).unwrap(),
            U256::ZERO
        );
        #[cfg(feature = "mpt-metrics")]
        assert_eq!(sparse.mpt_workload_metrics().storage_wipe_operations, 1);
    }

    #[test]
    fn pre_unzen_destroy_then_create2_redeploy_applies_delta_to_empty_storage() {
        let address = Address::repeat_byte(0x23);
        let (
            mut sparse,
            hashed_address,
            rewritten_slot,
            stale_parent_slot,
            hashed_rewritten_slot,
            _hashed_stale_parent_slot,
        ) = parent_with_storage(address);
        let code_hash = B256::repeat_byte(0x44);
        let redeployed = Account {
            nonce: 1,
            balance: U256::ZERO,
            bytecode_hash: Some(code_hash),
        };
        let recreated = HashedPostState::default()
            .with_accounts([(hashed_address, Some(redeployed))])
            .with_storages([(
                hashed_address,
                HashedStorage::from_iter([(hashed_rewritten_slot, U256::from(30))]),
            )]);

        let actual_root = sparse
            .calculate_state_root(recreated, &[hashed_address])
            .unwrap();
        assert_eq!(
            actual_root,
            expected_recreated_root(
                hashed_address,
                &redeployed,
                [(hashed_rewritten_slot, U256::from(30))],
            )
        );
        assert_eq!(
            sparse.storage(address, rewritten_slot).unwrap(),
            U256::from(30)
        );
        assert_eq!(
            sparse.storage(address, stale_parent_slot).unwrap(),
            U256::ZERO
        );
        #[cfg(feature = "mpt-metrics")]
        assert_eq!(sparse.mpt_workload_metrics().storage_wipe_operations, 1);
    }
}
