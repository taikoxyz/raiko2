use alloy_consensus::TrieAccount;
use alloy_primitives::{
    Address, B256, Bytes, KECCAK256_EMPTY, U256, keccak256,
    map::{AddressMap, B256Map},
};
use alloy_trie::EMPTY_ROOT_HASH;
use raiko2_primitives::{ExecutionWitness, StatelessValidationError, WitnessStateNode};
use reth_errors::ProviderError;
use reth_revm::state::Bytecode;
use reth_trie_common::HashedPostState;
use risc0_ethereum_trie::CachedTrie;
use std::{
    cell::RefCell,
    collections::hash_map::Entry,
    marker::PhantomData,
    panic::{AssertUnwindSafe, catch_unwind},
};

use crate::trie::{StatelessTrie, StatelessTrieExt};

/// Zero-overhead helper for tries that only contain RLP encoded data.
#[derive(Debug, Clone, Default)]
#[repr(transparent)]
struct RlpTrie<T> {
    inner: CachedTrie,
    phantom: PhantomData<T>,
}

impl<T: alloy_rlp::Decodable + alloy_rlp::Encodable> RlpTrie<T> {
    const fn new(inner: CachedTrie) -> Self {
        Self {
            inner,
            phantom: PhantomData,
        }
    }

    fn from_prehashed(
        root: B256,
        rlp_by_digest: &B256Map<impl AsRef<[u8]>>,
    ) -> alloy_rlp::Result<Self> {
        Ok(Self::new(CachedTrie::from_prehashed_nodes(
            root,
            rlp_by_digest,
        )?))
    }

    fn get(&self, key: impl AsRef<[u8]>) -> alloy_rlp::Result<Option<T>> {
        self.inner.get(key).map(alloy_rlp::decode_exact).transpose()
    }

    fn insert(&mut self, key: impl AsRef<[u8]>, value: T) {
        self.inner.insert(key, alloy_rlp::encode(value));
    }

    fn remove(&mut self, key: impl AsRef<[u8]>) -> bool {
        self.inner.remove(key)
    }

    fn hash(&mut self) -> B256 {
        self.inner.hash()
    }

    fn size(&self) -> usize {
        self.inner.size()
    }

    fn rlp_nodes(&self) -> Vec<Bytes> {
        self.inner.rlp_nodes()
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

    // all callers for invalid transaction validation
    callers: AddressMap<TrieAccount>,
}

impl SparseState {
    fn rlp_by_digest(
        witness: &ExecutionWitness,
        shared_state_nodes: &[WitnessStateNode],
    ) -> Result<B256Map<Bytes>, StatelessValidationError> {
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

    /// Removes an account from the state.
    fn remove_account(&mut self, hashed_address: &B256) {
        self.state.remove(hashed_address);
        self.storages.get_mut().remove(hashed_address);
    }

    /// Clears the storage of an account.
    fn clear_storage(&mut self, hashed_address: B256) -> &mut RlpTrie<U256> {
        self.storages
            .get_mut()
            .entry(hashed_address)
            .insert_entry(RlpTrie::default())
            .into_mut()
    }

    /// Returns a mutable version of the storage trie of the given account.
    fn storage_trie_mut(&mut self, hashed_address: B256) -> alloy_rlp::Result<&mut RlpTrie<U256>> {
        let trie = match self.storages.get_mut().entry(hashed_address) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                // build the storage trie matching the storage root of the account
                let storage_root = self
                    .state
                    .get(hashed_address)?
                    .map_or(EMPTY_ROOT_HASH, |a| a.storage_root);
                entry.insert(RlpTrie::from_prehashed(storage_root, &self.rlp_by_digest)?)
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
        let rlp_by_digest = Self::rlp_by_digest(witness, shared_state_nodes)?;

        // construct the state trie from the witness data and the given state root
        let state = RlpTrie::from_prehashed(pre_state_root, &rlp_by_digest)
            .map_err(|_| StatelessValidationError::WitnessRevealFailed { pre_state_root })?;

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
                callers: AddressMap::default(),
            },
            bytecode,
        ))
    }

    /// Returns the `TrieAccount` that corresponds to the `Address`.
    fn account(&self, address: Address) -> Result<Option<TrieAccount>, ProviderError> {
        let hashed_address = keccak256(address);
        let account = catch_unwind(AssertUnwindSafe(|| self.state.get(hashed_address))).map_err(
            |_| {
                ProviderError::TrieWitnessError(format!(
                    "state trie unresolved while loading account {address} (hashed {hashed_address})"
                ))
            },
        )??;

        match account {
            None => Ok(None),
            Some(account) => {
                // each time an account is accessed, check whether its storage trie already exists
                // otherwise construct it from the witness data and the account's storage root
                match self.storages.borrow_mut().entry(hashed_address) {
                    Entry::Vacant(entry) => {
                        entry.insert(RlpTrie::from_prehashed(
                            account.storage_root,
                            &self.rlp_by_digest,
                        )?);
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
        let value = catch_unwind(AssertUnwindSafe(|| storage_trie.get(hashed_slot))).map_err(
            |_| {
                ProviderError::TrieWitnessError(format!(
                    "storage trie unresolved while loading address {address} slot {slot} (hashed {hashed_slot})"
                ))
            },
        )??;
        Ok(value.unwrap_or(U256::ZERO))
    }

    /// Computes the new state root from the `HashedPostState`.
    fn calculate_state_root(
        &mut self,
        state: HashedPostState,
    ) -> Result<B256, StatelessValidationError> {
        let mut removed_accounts = Vec::new();
        for (hashed_address, account) in state.accounts {
            // nonexisting accounts must be removed from the state
            let Some(account) = account else {
                removed_accounts.push(hashed_address);
                continue;
            };

            // apply storage changes before computing the storage root
            let storage_root = match state.storages.get(&hashed_address) {
                None => self
                    .storage_trie_mut(hashed_address)
                    .map_err(|e| StatelessValidationError::StatelessExecutionFailed(e.to_string()))?
                    .hash(),
                Some(storage) => {
                    let storage_trie = if storage.wiped {
                        self.clear_storage(hashed_address)
                    } else {
                        self.storage_trie_mut(hashed_address).map_err(|e| {
                            StatelessValidationError::StatelessExecutionFailed(e.to_string())
                        })?
                    };

                    // apply all state modifications
                    for (hashed_key, value) in &storage.storage {
                        if !value.is_zero() {
                            storage_trie.insert(hashed_key, *value);
                        }
                    }
                    // removals must happen last, otherwise unresolved orphans might still exist
                    for (hashed_key, value) in &storage.storage {
                        if value.is_zero() {
                            storage_trie.remove(hashed_key);
                        }
                    }

                    storage_trie.hash()
                }
            };

            // update/insert the account after all changes have been processed
            let account = TrieAccount {
                nonce: account.nonce,
                balance: account.balance,
                storage_root,
                code_hash: account.bytecode_hash.unwrap_or(KECCAK256_EMPTY),
            };
            self.state.insert(hashed_address, account);
        }
        for hashed_address in &removed_accounts {
            self.remove_account(hashed_address);
        }

        Ok(self.state.hash())
    }
}

impl StatelessTrieExt for SparseState {
    fn append_callers(&mut self, callers: AddressMap<TrieAccount>) {
        for (addr, caller) in callers {
            self.callers.insert(addr, caller);
        }
    }
}
